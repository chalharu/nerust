//! Transient OSD queue: folding, merge/preempt/replace, burst caps.
//! Pure queue discipline; orchestration lives in [`OsdManager`](crate::session::osd::OsdManager).

use std::sync::Arc;
use std::time::{Duration, Instant};

use nerust_settings_core::model::local::NotificationSettings;

use crate::session::notification::{Message, NotificationLevel, ReplaceGroup, append_count};

#[derive(Debug, Clone)]
pub struct OsdRow {
    pub text: Arc<str>,
    pub rgb: (u8, u8, u8),
    pub tag: Arc<str>,
}

#[derive(Debug)]
pub(crate) struct OsdItem {
    pub(crate) row: OsdRow,
    pub(crate) level: NotificationLevel,
    /// Structured coalescing key: domain message plus count, never formatted text.
    pub(crate) message: crate::session::notification::Message,
    /// Rendered base text for the row rebuild; identity lives in `message`.
    pub(crate) base: std::sync::Arc<str>,
    pub(crate) count: u32,
    /// Display duration, stamped at promotion so a queued label always shows its full duration.
    pub(crate) duration: Duration,
    pub(crate) deadline: Instant,
    /// Latest-wins replace group ([`ReplaceGroup`]): at most one
    /// row per group exists. `None` rows always queue.
    pub(crate) group: Option<ReplaceGroup>,
}

/// Single sequential display (no stacking): one label at a time, FIFO by push order.
/// `pending` is capped so the total never exceeds three; merge/preempt/latest-wins apply.
#[derive(Debug, Default)]
pub(crate) struct TransientQueue {
    current: Option<OsdItem>,
    pending: std::collections::VecDeque<OsdItem>,
}

/// Push routing outcome: only a changed showing label dirties.
/// Queued rows just arm the deadline; promotion dirties on expiry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RouteEffect {
    ShowingChanged,
    QueuedBehind,
}

/// Merge target for a consecutive duplicate: the showing label
/// (visible change) or the queue tail (deadline refresh only).
#[derive(Debug, Clone, Copy)]
pub(crate) enum MergeTarget {
    Showing,
    Queued,
}

/// Which end the drop victim is taken from. Separate names keep
/// call sites readable; a bool would not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VictimEdge {
    /// Normal overflow: oldest row of the lowest severity present.
    Oldest,
    /// Demote insertion: newest row, so the just-demoted head survives.
    Newest,
}

/// Whether a composited frame will reach the screen. The OSD
/// channel is invisible without one, so rows fall back to modal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameChannel {
    Composited,
    Unpresented,
}

/// Modal fallback: without a composited frame only errors and warnings go modal; info drops.
/// A new channel must pick its route here.
pub(crate) fn falls_back_to_modal(
    kind: &Message,
    policy: &TransientPolicy,
    channel: FrameChannel,
) -> bool {
    match channel {
        FrameChannel::Unpresented => kind.wants_modal(),
        FrameChannel::Composited => kind.level() == NotificationLevel::Error && policy.error_modal,
    }
}

/// Narrow display policy for transient rows; decision code takes this, never the settings surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransientPolicy {
    pub info_enabled: bool,
    pub warning_enabled: bool,
    pub error_enabled: bool,
    pub info_duration_ms: u32,
    pub warning_duration_ms: u32,
    pub info_rgb: (u8, u8, u8),
    pub warning_rgb: (u8, u8, u8),
    pub error_rgb: (u8, u8, u8),
    pub error_modal: bool,
}

impl From<&NotificationSettings> for TransientPolicy {
    fn from(settings: &NotificationSettings) -> Self {
        Self {
            info_enabled: settings.info_enabled,
            warning_enabled: settings.warning_enabled,
            error_enabled: settings.error_enabled,
            info_duration_ms: settings.info_duration_ms,
            warning_duration_ms: settings.warning_duration_ms,
            info_rgb: settings.info_color.rgb(),
            warning_rgb: settings.warning_color.rgb(),
            error_rgb: settings.error_color.rgb(),
            error_modal: settings.error_modal,
        }
    }
}

impl TransientPolicy {
    /// Whether `level` rows display at all.
    pub(crate) fn enabled(&self, level: NotificationLevel) -> bool {
        match level {
            NotificationLevel::Info => self.info_enabled,
            NotificationLevel::Warning => self.warning_enabled,
            NotificationLevel::Error => self.error_enabled,
        }
    }

    /// Display duration for `level` rows.
    pub(crate) fn duration(&self, level: NotificationLevel) -> Duration {
        let raw = match level {
            NotificationLevel::Info => self.info_duration_ms,
            NotificationLevel::Warning => self.warning_duration_ms,
            NotificationLevel::Error => 5_000,
        };
        Duration::from_millis(u64::from(raw.clamp(500, 60_000)))
    }

    /// Display color for `level` rows.
    pub(crate) fn color(&self, level: NotificationLevel) -> (u8, u8, u8) {
        match level {
            NotificationLevel::Info => self.info_rgb,
            NotificationLevel::Warning => self.warning_rgb,
            NotificationLevel::Error => self.error_rgb,
        }
    }
}

pub(crate) fn counted_row(base: &str, count: u32, rgb: (u8, u8, u8), tag: Arc<str>) -> OsdRow {
    OsdRow {
        text: append_count(base.to_string(), count).into(),
        rgb,
        tag,
    }
}

impl TransientQueue {
    /// Two pending rows plus the single showing row.
    const MAX_QUEUED: usize = 2;

    /// Re-project retained domain messages under current presentation policy.
    pub(crate) fn refresh(
        &mut self,
        policy: &TransientPolicy,
        language: nerust_settings_core::model::language::AppLanguage,
        now: Instant,
        channel: FrameChannel,
    ) -> Vec<(
        crate::session::notification::Message,
        u32,
        NotificationLevel,
    )> {
        let renderer = crate::session::notification::NotificationRenderer { language };
        let mut modal = Vec::new();
        let mut update = |item: &mut OsdItem| {
            if !policy.enabled(item.level) {
                return false;
            }
            item.base = renderer.render_message(&item.message).into();
            item.row = counted_row(
                &item.base,
                item.count,
                policy.color(item.level),
                Arc::clone(&item.row.tag),
            );
            item.duration = policy.duration(item.level);
            item.deadline = item.deadline.min(now + item.duration);
            if channel == FrameChannel::Unpresented
                || (item.level == NotificationLevel::Error && policy.error_modal)
            {
                if item.level != NotificationLevel::Info {
                    modal.push((item.message.clone(), item.count, item.level));
                }
                return false;
            }
            true
        };
        if let Some(current) = self.current.as_mut()
            && !update(current)
        {
            self.current = None;
        }
        self.pending.retain_mut(&mut update);
        modal
    }

    /// Lowest-severity index to drop on overflow, never an Error while a lower severity row exists.
    /// Severity rank primary, queue position secondary.
    fn victim_index(
        pending: &std::collections::VecDeque<OsdItem>,
        edge: VictimEdge,
    ) -> Option<usize> {
        fn rank(level: NotificationLevel) -> u8 {
            match level {
                NotificationLevel::Info => 0,
                NotificationLevel::Warning => 1,
                NotificationLevel::Error => 2,
            }
        }
        let position = |index: usize| match edge {
            VictimEdge::Oldest => index,
            VictimEdge::Newest => usize::MAX - index,
        };
        pending
            .iter()
            .enumerate()
            .filter(|(_, item)| item.level != NotificationLevel::Error)
            .min_by_key(|(index, item)| (rank(item.level), position(*index)))
            .map(|(index, _)| index)
    }

    /// Drop the overflow victim, never dropping an Error while a
    /// lower severity row exists.
    fn evict_overflow(pending: &mut std::collections::VecDeque<OsdItem>, edge: VictimEdge) {
        match Self::victim_index(pending, edge) {
            Some(index) => {
                pending.remove(index);
            }
            None => {
                pending.pop_front();
            }
        }
    }

    /// Merge a consecutive duplicate into the showing label or the queue tail (×N, timer reset).
    /// Reports where the merge landed for dirty gating.
    fn merge_consecutive(&mut self, item: &OsdItem) -> Option<MergeTarget> {
        if !self.pending.is_empty() {
            // Key-only merge: collisions only over-coalesce, never misroute.
            let matches = self.pending.back().is_some_and(|back| {
                back.level == item.level
                    && back.message.coalescing_key() == item.message.coalescing_key()
            });
            if !matches {
                return None;
            }
            let back = self.pending.back_mut()?;
            back.count += item.count;
            back.row = counted_row(
                &item.base,
                back.count,
                item.row.rgb,
                Arc::clone(&item.row.tag),
            );
            back.deadline = item.deadline;
            return Some(MergeTarget::Queued);
        }
        let current = self.current.as_mut()?;
        if current.level != item.level
            || current.message.coalescing_key() != item.message.coalescing_key()
        {
            return None;
        }
        current.count += item.count;
        current.row = counted_row(
            &item.base,
            current.count,
            item.row.rgb,
            Arc::clone(&item.row.tag),
        );
        current.deadline = item.deadline;
        Some(MergeTarget::Showing)
    }

    /// Error rows preempt a non-Error showing label; the demoted label waits with duration intact.
    /// Returns the row unhandled when no preempt applies, so the caller keeps ownership by move.
    fn preempt_for_error(&mut self, item: OsdItem) -> Option<OsdItem> {
        if item.level != NotificationLevel::Error {
            return Some(item);
        }
        if self
            .current
            .as_ref()
            .is_none_or(|current| current.level == NotificationLevel::Error)
        {
            return Some(item);
        }
        // Showing label confirmed non-error above; the `else` covers only the impossible case.
        let Some(demoted) = self.current.take() else {
            return Some(item);
        };
        self.current = Some(item);
        self.pending.push_front(demoted);
        while self.pending.len() > Self::MAX_QUEUED {
            Self::evict_overflow(&mut self.pending, VictimEdge::Newest);
        }
        None
    }

    /// Grouped rows collapse latest-wins in place instead of queueing.
    /// `Ok` reports whether the showing label changed; `Err` returns the row unhandled.
    fn replace_grouped(&mut self, item: OsdItem) -> Result<RouteEffect, Box<OsdItem>> {
        let Some(group) = item.group else {
            return Err(Box::new(item));
        };
        if let Some(current) = self.current.as_mut()
            && current.group == Some(group)
        {
            *current = item;
            return Ok(RouteEffect::ShowingChanged);
        }
        if let Some(existing) = self.pending.iter_mut().find(|row| row.group == Some(group)) {
            *existing = item;
            return Ok(RouteEffect::QueuedBehind);
        }
        Err(Box::new(item))
    }

    /// Route one staged row through merge, preempt, and replace policies, queueing what survives.
    pub(crate) fn route_row(&mut self, item: OsdItem) -> RouteEffect {
        let merged = self.merge_consecutive(&item);
        match merged {
            Some(MergeTarget::Showing) => return RouteEffect::ShowingChanged,
            Some(MergeTarget::Queued) => return RouteEffect::QueuedBehind,
            None => {}
        }
        let Some(item) = self.preempt_for_error(item) else {
            return RouteEffect::ShowingChanged;
        };
        let item = match self.replace_grouped(item) {
            Ok(effect) => return effect,
            Err(item) => *item,
        };
        self.pending.push_back(item);
        // Full queue drops the lowest severity first; Errors drop
        // only when every queued row is an Error.
        while self.pending.len() > Self::MAX_QUEUED {
            Self::evict_overflow(&mut self.pending, VictimEdge::Oldest);
        }
        RouteEffect::QueuedBehind
    }

    /// Promote pending when idle, or advance on expiry. True only
    /// on a visible change; unchanged steady state stays false.
    pub(crate) fn poll(&mut self, now: Instant) -> bool {
        let need = match &self.current {
            None => !self.pending.is_empty(),
            Some(current) => current.deadline <= now,
        };
        if !need {
            return false;
        }
        // Errors outrank queued information even before the first row is shown.
        let next = self
            .pending
            .iter()
            .position(|item| item.level == NotificationLevel::Error)
            .and_then(|index| self.pending.remove(index))
            .or_else(|| self.pending.pop_front());
        if let Some(mut item) = next {
            // Fresh full duration from the moment it shows.
            item.deadline = now + item.duration;
            self.current = Some(item);
        } else {
            self.current = None;
        }
        true
    }

    /// Borrowed showing row for the sync emptiness check; only `Render` builds the owned plan.
    pub(crate) fn rows(&self) -> &[OsdRow] {
        self.current
            .as_ref()
            .map(|current| std::slice::from_ref(&current.row))
            .unwrap_or(&[])
    }

    pub(crate) fn visible_row(&self) -> Option<&OsdRow> {
        self.current.as_ref().map(|current| &current.row)
    }

    pub(crate) fn has_visible(&self) -> bool {
        self.current.is_some()
    }

    /// OSD expiry deadline for the loop wait, on the caller's tick
    /// clock (no second clock read inside).
    pub(crate) fn wakeup_deadline(&self, now: Instant) -> Option<Instant> {
        match &self.current {
            Some(current) => Some(current.deadline),
            // Pending work with nothing showing: wake immediately.
            None if !self.pending.is_empty() => Some(now),
            None => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::notification::{NotificationLevel, ReplaceGroup};
    use nerust_settings_core::model::local::NotificationSettings;
    use std::time::{Duration, Instant};

    fn settings() -> NotificationSettings {
        NotificationSettings::default()
    }

    #[test]
    fn transient_policy_mirrors_settings_surface() {
        let settings = settings();
        let policy = TransientPolicy::from(&settings);
        assert_eq!(policy.info_enabled, settings.info_enabled);
        assert_eq!(policy.warning_enabled, settings.warning_enabled);
        assert_eq!(policy.error_enabled, settings.error_enabled);
        assert_eq!(policy.error_modal, settings.error_modal);
        assert_eq!(policy.info_rgb, settings.info_color.rgb());
        assert_eq!(policy.error_rgb, settings.error_color.rgb());
    }

    #[test]
    fn falls_back_to_modal_covers_levels_and_channels() {
        use crate::session::notification::Message as K;
        let mut modal = settings();
        modal.error_modal = true;
        let mut osd = settings();
        osd.error_modal = false;
        let error = K::slot_failed(Some(1), crate::session::error::SlotOpFailure::Storage);
        let warning = K::slot_failed(None, crate::session::error::SlotOpFailure::Missing);
        // Composited: only Error with modal enabled.
        assert!(falls_back_to_modal(
            &error,
            &TransientPolicy::from(&modal),
            FrameChannel::Composited
        ));
        assert!(!falls_back_to_modal(
            &error,
            &TransientPolicy::from(&osd),
            FrameChannel::Composited
        ));
        assert!(!falls_back_to_modal(
            &warning,
            &TransientPolicy::from(&modal),
            FrameChannel::Composited
        ));
        assert!(!falls_back_to_modal(
            &K::paused(),
            &TransientPolicy::from(&modal),
            FrameChannel::Composited
        ));
        // Unpresented: Error and Warning go modal even with it off.
        assert!(falls_back_to_modal(
            &error,
            &TransientPolicy::from(&modal),
            FrameChannel::Unpresented
        ));
        assert!(falls_back_to_modal(
            &warning,
            &TransientPolicy::from(&modal),
            FrameChannel::Unpresented
        ));
        assert!(falls_back_to_modal(
            &error,
            &TransientPolicy::from(&osd),
            FrameChannel::Unpresented
        ));
        assert!(falls_back_to_modal(
            &warning,
            &TransientPolicy::from(&osd),
            FrameChannel::Unpresented
        ));
        // Unpresented: Info is dropped, never modal.
        assert!(!falls_back_to_modal(
            &K::paused(),
            &TransientPolicy::from(&modal),
            FrameChannel::Unpresented
        ));
        assert!(!falls_back_to_modal(
            &K::paused(),
            &TransientPolicy::from(&osd),
            FrameChannel::Unpresented
        ));
    }

    fn test_item(level: NotificationLevel, key: &str, now: Instant) -> OsdItem {
        use crate::session::notification::{Message, MessageKey};
        let message = match key {
            "a" => Message::unit(MessageKey::Paused),
            "b" => Message::unit(MessageKey::Resumed),
            "info" => Message::unit(MessageKey::Reset),
            other => Message::slot(
                MessageKey::SlotSaved,
                Some(other.len() as u64 + other.bytes().fold(0u64, |a, b| a + b as u64)),
            ),
        };
        OsdItem {
            row: counted_row(key, 1, (200, 200, 200), Arc::from("[I]")),
            level,
            message,
            base: key.into(),
            count: 1,
            duration: Duration::from_millis(2_000),
            deadline: now + Duration::from_millis(2_000),
            group: None,
        }
    }

    #[test]
    fn merge_lands_on_showing_row_and_bumps_count() {
        let mut queue = TransientQueue::default();
        let now = Instant::now();
        assert_eq!(
            queue.route_row(test_item(NotificationLevel::Info, "a", now)),
            RouteEffect::QueuedBehind
        );
        assert!(queue.poll(now));
        assert_eq!(
            queue.route_row(test_item(NotificationLevel::Info, "a", now)),
            RouteEffect::ShowingChanged
        );
        let rows = queue.rows();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].text.contains("×2"));
    }

    #[test]
    fn error_is_promoted_before_already_queued_info() {
        let mut queue = TransientQueue::default();
        let now = Instant::now();
        queue.route_row(test_item(NotificationLevel::Info, "info", now));
        queue.route_row(test_item(NotificationLevel::Error, "boom", now));
        assert!(queue.poll(now));
        assert_eq!(
            queue.current.as_ref().unwrap().level,
            NotificationLevel::Error
        );
        assert!(queue.poll(now + Duration::from_millis(2_001)));
        assert_eq!(
            queue.current.as_ref().unwrap().level,
            NotificationLevel::Info
        );
    }

    #[test]
    fn interleaved_identity_breaks_merge_run() {
        let mut queue = TransientQueue::default();
        let now = Instant::now();
        assert_eq!(
            queue.route_row(test_item(NotificationLevel::Info, "a", now)),
            RouteEffect::QueuedBehind
        );
        assert!(queue.poll(now));
        assert_eq!(
            queue.route_row(test_item(NotificationLevel::Info, "b", now)),
            RouteEffect::QueuedBehind
        );
        // Last accepted is "b": the rerun of "a" must neither merge
        // into the tail nor into the showing label.
        assert_eq!(
            queue.route_row(test_item(NotificationLevel::Info, "a", now)),
            RouteEffect::QueuedBehind
        );
        assert_eq!(
            queue.visible_row().map(|row| row.text.to_string()),
            Some("a".to_string())
        );
    }

    #[test]
    fn error_preempts_showing_info_and_keeps_it_head() {
        let mut queue = TransientQueue::default();
        let now = Instant::now();
        queue.route_row(test_item(NotificationLevel::Info, "info", now));
        assert!(queue.poll(now));
        assert_eq!(
            queue.route_row(test_item(NotificationLevel::Error, "boom", now)),
            RouteEffect::ShowingChanged
        );
        assert!(
            queue
                .visible_row()
                .map(|row| row.text.contains("boom"))
                .unwrap_or(false)
        );
        // Demoted info waits at the head: promoted next once the error clears.
        assert!(queue.poll(now + Duration::from_millis(2_001)));
        assert!(
            queue
                .visible_row()
                .map(|row| row.text.contains("info"))
                .unwrap_or(false)
        );
    }

    #[test]
    fn non_error_never_preempts_and_error_never_preempts_error() {
        let mut queue = TransientQueue::default();
        let now = Instant::now();
        queue.route_row(test_item(NotificationLevel::Info, "info", now));
        assert!(queue.poll(now));
        assert_eq!(
            queue.route_row(test_item(NotificationLevel::Warning, "warn", now)),
            RouteEffect::QueuedBehind
        );
        let mut errors = TransientQueue::default();
        errors.route_row(test_item(NotificationLevel::Error, "first", now));
        assert!(errors.poll(now));
        assert_eq!(
            errors.route_row(test_item(NotificationLevel::Error, "second", now)),
            RouteEffect::QueuedBehind
        );
    }

    #[test]
    fn grouped_rows_replace_latest_wins() {
        let mut queue = TransientQueue::default();
        let now = Instant::now();
        let mut first = test_item(NotificationLevel::Info, "first", now);
        first.group = Some(ReplaceGroup::SelectedSlot);
        let mut second = test_item(NotificationLevel::Info, "second", now);
        second.group = Some(ReplaceGroup::SelectedSlot);
        assert_eq!(queue.route_row(first), RouteEffect::QueuedBehind);
        assert_eq!(queue.route_row(second), RouteEffect::QueuedBehind);
        assert!(queue.poll(now));
        assert_eq!(
            queue.visible_row().map(|row| row.text.to_string()),
            Some("second".to_string())
        );
    }

    #[test]
    fn pending_queue_keeps_at_most_two_rows() {
        let mut queue = TransientQueue::default();
        let now = Instant::now();
        for key in ["a", "b", "info"] {
            queue.route_row(test_item(NotificationLevel::Info, key, now));
        }
        assert_eq!(TransientQueue::MAX_QUEUED, 2);
        assert_eq!(queue.pending.len(), 2);
        assert_eq!(queue.pending.front().unwrap().base.as_ref(), "b");
        assert_eq!(queue.pending.back().unwrap().base.as_ref(), "info");
    }

    #[test]
    fn overflow_evicts_lowest_severity_first() {
        let mut pending = std::collections::VecDeque::new();
        let now = Instant::now();
        pending.push_back(test_item(NotificationLevel::Info, "i1", now));
        pending.push_back(test_item(NotificationLevel::Warning, "w1", now));
        pending.push_back(test_item(NotificationLevel::Error, "e1", now));
        TransientQueue::evict_overflow(&mut pending, VictimEdge::Oldest);
        assert!(pending.iter().all(|item| item.base.as_ref() != "i1"));
        assert_eq!(pending.len(), 2);
        TransientQueue::evict_overflow(&mut pending, VictimEdge::Newest);
        assert!(pending.iter().all(|item| item.base.as_ref() != "w1"));
    }

    #[test]
    fn poll_promotes_on_demand_and_reports_change() {
        let mut queue = TransientQueue::default();
        let now = Instant::now();
        assert!(!queue.poll(now));
        assert!(!queue.has_visible());
        queue.route_row(test_item(NotificationLevel::Info, "a", now));
        assert!(queue.poll(now));
        assert!(queue.has_visible());
        // Steady before expiry: quiet.
        assert!(!queue.poll(now));
        // Past the duration the label clears.
        assert!(queue.poll(now + Duration::from_millis(2_001)));
        assert!(!queue.has_visible());
    }

    #[test]
    fn wakeup_deadline_distinguishes_idle_queued_and_showing() {
        let mut queue = TransientQueue::default();
        let now = Instant::now();
        assert_eq!(queue.wakeup_deadline(now), None);
        queue.route_row(test_item(NotificationLevel::Info, "a", now));
        assert_eq!(queue.wakeup_deadline(now), Some(now));
        assert!(queue.poll(now));
        assert_eq!(
            queue.wakeup_deadline(now),
            Some(now + Duration::from_millis(2_000))
        );
    }
}

#[cfg(test)]
mod transient_cov_tests {
    use crate::session::osd_transient::*;

    #[test]
    fn policy_gates_levels_durations_and_colors() {
        let policy = TransientPolicy::from(
            &nerust_settings_core::model::local::NotificationSettings::default(),
        );
        for level in [
            NotificationLevel::Info,
            NotificationLevel::Warning,
            NotificationLevel::Error,
        ] {
            let _ = policy.enabled(level);
            assert!(policy.duration(level) >= Duration::from_millis(500));
            let _ = policy.color(level);
        }
        assert_eq!(
            counted_row("a", 1, (1, 2, 3), Arc::from("[INFO]"))
                .text
                .as_ref(),
            "a"
        );
        assert_eq!(
            counted_row("a", 2, (1, 2, 3), Arc::from("[INFO]"))
                .text
                .as_ref(),
            "a ×2"
        );
    }

    #[test]
    fn modal_fallback_routes_by_channel() {
        use crate::session::notification::Message;
        let policy = TransientPolicy::from(
            &nerust_settings_core::model::local::NotificationSettings::default(),
        );
        assert!(falls_back_to_modal(
            &Message::persistence_unavailable(),
            &policy,
            FrameChannel::Unpresented
        ));
        assert!(!falls_back_to_modal(
            &Message::paused(),
            &policy,
            FrameChannel::Unpresented
        ));
        let _ = falls_back_to_modal(&Message::paused(), &policy, FrameChannel::Composited);
    }
}

//! OSD display state shared by frontends: queue folding, expiry,
//! burst caps, strip math, and modal aggregation. Frontends render
//! `rows()` and honor `wakeup_deadline()`; all policy lives here so
//! tao/gtk/android behave identically.

use nerust_render_traits::renderer::OsdAnchor;
use std::time::{Duration, Instant};

use nerust_settings_core::model::language::AppLanguage;
use nerust_settings_core::model::local::{
    NotificationAreaMode, NotificationPosition, NotificationSettings,
};

use super::notification::{
    NotificationKind, NotificationLevel, ReplaceGroup, UiNotification, append_count,
};

/// Level tag prefixing OSD labels: text (not color or shape
/// alone) carries severity. History uses [`level_name`]
/// instead: the grid Level column and the OSD tag are managed
/// separately by design (review feedback).
/// Single source for level text: tag and grid name side by
/// side so a new level cannot update one and forget the other.
struct LevelText {
    tag: &'static str,
    name: &'static str,
}

fn level_text(level: NotificationLevel) -> LevelText {
    match level {
        NotificationLevel::Info => LevelText {
            tag: "[INFO]",
            name: "Info",
        },
        NotificationLevel::Warning => LevelText {
            tag: "[WARN]",
            name: "Warning",
        },
        NotificationLevel::Error => LevelText {
            tag: "[ERROR]",
            name: "Error",
        },
    }
}

pub fn level_tag(level: NotificationLevel) -> &'static str {
    level_text(level).tag
}

/// Full level name for the history grid Level column.
pub fn level_name(level: NotificationLevel) -> &'static str {
    level_text(level).name
}

/// Localized level name for history grid and filter options.
/// `Error` maps to `UiText::Error` for column width.
pub fn level_label(
    level: NotificationLevel,
    language: nerust_settings_core::model::language::AppLanguage,
) -> &'static str {
    use nerust_settings_core::i18n::{UiText, text as ui_text};
    match level {
        NotificationLevel::Info => ui_text(language, UiText::NotificationInfo),
        NotificationLevel::Warning => ui_text(language, UiText::NotificationWarning),
        NotificationLevel::Error => ui_text(language, UiText::Error),
    }
}

/// One rendered history line: datetime plus grid columns.
/// `module` is the log module so rows grep the `notification:` records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryLine {
    /// YYYY-MM-DD HH:MM:SS (local wall-clock) of the push.
    pub datetime: String,
    pub level: NotificationLevel,
    pub module: &'static str,
    pub text: String,
    pub count: u32,
}

/// Local wall-clock datetime from a `SystemTime`.
/// Falls back to UTC when the zone is indeterminate.
pub fn format_datetime(time: std::time::SystemTime) -> String {
    format_datetime_in(time, local_utc_offset_secs())
}

/// `SystemTime` as local epoch seconds for [`format_datetime`]:
/// the UTC instant plus the platform zone offset. Mirrors the
/// persistence slot-time precedent (`persistence::time`).
fn local_epoch_secs(time: std::time::SystemTime) -> i64 {
    let utc = time
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    (utc as i64).saturating_add(local_utc_offset_secs())
}

/// Platform zone offset in seconds, resolved once per sync — not per
/// row. Falls back to UTC when the zone is indeterminate.
fn local_utc_offset_secs() -> i64 {
    time::UtcOffset::current_local_offset()
        .map(|o| i64::from(o.whole_seconds()))
        .unwrap_or(0)
}

/// Datetime with a pre-resolved offset (see
/// [`local_utc_offset_secs`]).
fn format_datetime_in(time: std::time::SystemTime, offset_secs: i64) -> String {
    let utc = time
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Floor-divide so pre-1970 instants keep a non-negative clock.
    let secs = (utc as i64).saturating_add(offset_secs);
    let days = secs.div_euclid(86_400);
    let clock = secs.rem_euclid(86_400);
    // Howard Hinnant's civil-from-days; 719468 = days to 1970-01-01.
    let era = (days + 719_468) / 146_097;
    let doe = (days + 719_468) - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let full_year = (year + i64::from(month <= 2)) as u32;
    format!(
        "{full_year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}",
        clock / 3600,
        clock / 60 % 60,
        clock % 60
    )
}

/// Build render-ready history lines in one pass, resolving the zone
/// offset once for all rows. Lifecycle delegates here so every
/// frontend shares identical rows without duplicating the mapping.
pub fn history_lines<'a>(
    history: impl Iterator<Item = &'a UiNotification>,
    language: AppLanguage,
) -> Vec<HistoryLine> {
    let offset_secs = local_utc_offset_secs();
    history
        .map(|n| {
            let text = super::notification::render_text(&n.kind, n.count, language);
            HistoryLine {
                datetime: format_datetime_in(n.at, offset_secs),
                level: n.level,
                module: super::notification::LOG_MODULE,
                text,
                count: n.count,
            }
        })
        .collect()
}

pub fn format_time(time: std::time::SystemTime) -> String {
    let clock = local_epoch_secs(time).rem_euclid(86_400);
    format!(
        "{:02}:{:02}:{:02}",
        clock / 3600,
        clock / 60 % 60,
        clock % 60
    )
}

#[derive(Debug, Clone)]
pub struct OsdRow {
    pub text: String,
    pub rgb: (u8, u8, u8),
    pub tag: &'static str,
}

#[derive(Debug)]
struct OsdItem {
    row: OsdRow,
    level: NotificationLevel,
    /// Identity for coalescing (text before the `×N` suffix).
    key: String,
    count: u32,
    /// Display duration: the deadline is stamped at promotion so a
    /// queued label always shows for its full duration instead of
    /// carrying the push-time remainder.
    duration: Duration,
    deadline: Instant,
    /// Latest-wins replace group ([`ReplaceGroup`]): at most one
    /// row per group exists. `None` rows always queue.
    group: Option<ReplaceGroup>,
}

/// Single sequential display (no stacking): one label at a time,
/// FIFO by push order. `pending` is capped at two so the total
/// (current + pending) never exceeds three.
const MAX_TRANSIENT_QUEUED: usize = 2;

/// Transient in-frame queue: showing row plus pending rows with the
/// merge/preempt/latest-wins discipline. No modal state.
#[derive(Debug, Default)]
struct TransientQueue {
    current: Option<OsdItem>,
    pending: std::collections::VecDeque<OsdItem>,
    /// Last accepted notification identity. Merging (`×N`) applies
    /// only to strictly consecutive duplicates; anything interleaved
    /// queues separately so flapping states keep full slots.
    last_key: Option<(NotificationLevel, String)>,
}

/// Modal error batches with sequential display pumping.
#[derive(Debug, Default)]
struct ModalBox {
    batch: Vec<String>,
    queue: std::collections::VecDeque<String>,
    open: bool,
    /// Total overflow losses. Pure fact, no I/O: the pump adapter
    /// decides logging from the delta (see `pump_modal`).
    dropped_total: u64,
}

/// Notification display state: transient in-frame rows plus modal batches.
/// Queue discipline lives in TransientQueue, pumping in ModalBox.
#[derive(Debug, Default)]
pub struct OsdManager {
    transient: TransientQueue,
    modal_box: ModalBox,
    /// Monotonic display-generation. Presenters remember the last
    /// generation they synced; inequality means re-render.
    generation: u64,
    /// Overflow warn episode flag. Re-arms when the modal queue
    /// fully drains (see the modal source below).
    modal_warned_full: bool,
}

/// Write side of the modal queue. Only the shell pump and the
/// owning frontend adapter implement this; renderers never see it.
pub trait ModalSink {
    fn queue_modal(&mut self, detail: String);
    fn requeue_front(&mut self, detail: String);
}

/// Read side of the modal queue. Dialog hosts take batches and
/// ack them; they cannot inject or reorder.
pub trait ModalSource {
    fn take_modal_batch(&mut self) -> Option<String>;
    fn modal_closed(&mut self);
    fn modal_pending(&self) -> usize;
}

/// Push routing outcome: only a changed showing label dirties.
/// Queued rows just arm the deadline; promotion dirties on expiry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RouteEffect {
    ShowingChanged,
    QueuedBehind,
}

/// Merge target for a consecutive duplicate: the showing label
/// (visible change) or the queue tail (deadline refresh only).
#[derive(Debug, Clone, Copy)]
enum MergeTarget {
    Showing,
    Queued,
}

/// Which end the drop victim is taken from. Separate names keep
/// call sites readable; a bool would not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VictimEdge {
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

/// Modal fallback: without a composited frame only modal-worthy
/// kinds (errors, warnings) go modal; info is dropped. Exhaustive:
/// a new channel must pick its route here.
fn falls_back_to_modal(
    kind: &NotificationKind,
    policy: &TransientPolicy,
    channel: FrameChannel,
) -> bool {
    match channel {
        FrameChannel::Unpresented => kind.wants_modal(),
        FrameChannel::Composited => kind.level() == NotificationLevel::Error && policy.error_modal,
    }
}

/// Narrow display policy for transient rows. Decision code takes
/// this instead of the full settings surface: a new settings field
/// cannot ripple into OSD routing.
#[derive(Debug, Clone, Copy)]
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

/// Display-policy helpers using a narrow policy and row data only.
fn enabled(level: NotificationLevel, policy: &TransientPolicy) -> bool {
    match level {
        NotificationLevel::Info => policy.info_enabled,
        NotificationLevel::Warning => policy.warning_enabled,
        NotificationLevel::Error => policy.error_enabled,
    }
}

fn duration_ms(level: NotificationLevel, policy: &TransientPolicy) -> u64 {
    let raw = match level {
        NotificationLevel::Info => policy.info_duration_ms,
        NotificationLevel::Warning => policy.warning_duration_ms,
        NotificationLevel::Error => 5_000,
    };
    u64::from(raw.clamp(500, 60_000))
}

fn counted_row(base: String, count: u32, rgb: (u8, u8, u8), tag: &'static str) -> OsdRow {
    OsdRow {
        text: append_count(base, count),
        rgb,
        tag,
    }
}

fn color(level: NotificationLevel, policy: &TransientPolicy) -> (u8, u8, u8) {
    match level {
        NotificationLevel::Info => policy.info_rgb,
        NotificationLevel::Warning => policy.warning_rgb,
        NotificationLevel::Error => policy.error_rgb,
    }
}

impl TransientQueue {
    fn victim_index(
        pending: &std::collections::VecDeque<OsdItem>,
        edge: VictimEdge,
    ) -> Option<usize> {
        let scan = |level| match edge {
            VictimEdge::Oldest => pending.iter().position(|item| item.level == level),
            VictimEdge::Newest => pending.iter().rposition(|item| item.level == level),
        };
        scan(NotificationLevel::Info).or_else(|| scan(NotificationLevel::Warning))
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

    /// Merge a consecutive duplicate into the showing label or the
    /// queue tail (`×N`, timer reset). Anything interleaved breaks the
    /// run, so pause/resume flaps keep full slots. Consecutiveness is
    /// derived here from the last accepted identity, so callers pass
    /// only the row. Reports where the merge landed for dirty gating.
    fn merge_consecutive(&mut self, item: &OsdItem) -> Option<MergeTarget> {
        let consecutive = self
            .last_key
            .as_ref()
            .is_some_and(|(level, key)| *level == item.level && key == &item.key);
        if !consecutive {
            return None;
        }
        if let Some(current) = self.current.as_mut()
            && current.level == item.level
            && current.key == item.key
        {
            current.count += item.count;
            current.row = counted_row(item.key.clone(), current.count, item.row.rgb, item.row.tag);
            current.deadline = item.deadline;
            return Some(MergeTarget::Showing);
        }
        if let Some(back) = self.pending.back_mut()
            && back.level == item.level
            && back.key == item.key
        {
            back.count += item.count;
            back.row = counted_row(item.key.clone(), back.count, item.row.rgb, item.row.tag);
            back.deadline = item.deadline;
            return Some(MergeTarget::Queued);
        }
        None
    }

    /// Error rows preempt a non-Error showing label (Warnings wait
    /// their turn). The demoted label waits at the queue head with
    /// its duration intact. Returns the row unhandled when no
    /// preempt applies, so the caller keeps ownership by move.
    fn preempt_for_error(&mut self, item: OsdItem) -> Option<OsdItem> {
        if item.level != NotificationLevel::Error {
            return Some(item);
        }
        let Some(current) = self.current.as_ref() else {
            return Some(item);
        };
        if current.level == NotificationLevel::Error {
            return Some(item);
        }
        let demoted = self.current.take().expect("showing label checked");
        self.current = Some(item);
        self.pending.push_front(demoted);
        while self.pending.len() > MAX_TRANSIENT_QUEUED {
            Self::evict_overflow(&mut self.pending, VictimEdge::Newest);
        }
        None
    }

    /// Grouped rows collapse latest-wins in place: a row replaces the
    /// showing or queued row of the same group instead of queueing.
    /// `Ok` reports whether the showing label changed; `Err` returns
    /// the row unhandled when no row of its group exists yet.
    fn replace_grouped(&mut self, item: OsdItem) -> Result<RouteEffect, OsdItem> {
        let Some(group) = item.group else {
            return Err(item);
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
        Err(item)
    }

    /// Route one staged row through merge, preempt, and replace
    /// policies, queuing what survives. Ownership stays inside this
    /// dispatcher so helpers move rows instead of cloning them.
    fn route_row(&mut self, item: OsdItem) -> RouteEffect {
        let merged = self.merge_consecutive(&item);
        self.last_key = Some((item.level, item.key.clone()));
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
            Err(item) => item,
        };
        self.pending.push_back(item);
        // Full queue drops the lowest severity first; Errors drop
        // only when every queued row is an Error.
        while self.pending.len() > MAX_TRANSIENT_QUEUED {
            Self::evict_overflow(&mut self.pending, VictimEdge::Oldest);
        }
        RouteEffect::QueuedBehind
    }

    /// Promote pending when idle, or advance on expiry. True only
    /// on a visible change; unchanged steady state stays false.
    fn poll(&mut self, now: Instant) -> bool {
        let need = match &self.current {
            None => !self.pending.is_empty(),
            Some(current) => current.deadline <= now,
        };
        if !need {
            return false;
        }
        if let Some(mut item) = self.pending.pop_front() {
            // Fresh full duration from the moment it shows.
            item.deadline = now + item.duration;
            self.current = Some(item);
        } else {
            self.current = None;
        }
        true
    }

    /// True when display state changed since the last sync.
    fn rows(&self) -> Vec<OsdRow> {
        self.current
            .as_ref()
            .map(|current| vec![current.row.clone()])
            .unwrap_or_default()
    }

    fn has_visible(&self) -> bool {
        self.current.is_some()
    }

    /// OSD expiry deadline for the loop wait, on the caller's tick
    /// clock (no second clock read inside).
    fn wakeup_deadline(&self, now: Instant) -> Option<Instant> {
        match &self.current {
            Some(current) => Some(current.deadline),
            // Pending work with nothing showing: wake immediately.
            None if !self.pending.is_empty() => Some(now),
            None => None,
        }
    }
}

impl ModalBox {
    /// Cap shared by queue and requeue paths (drop-oldest discipline).
    const MAX_QUEUED: usize = 8;

    fn push(&mut self, text: String) {
        self.batch.push(text);
    }

    fn take_batch(&mut self) -> Vec<String> {
        std::mem::take(&mut self.batch)
    }

    /// Take pending error lines as one display batch, bulleted.
    /// Single source for the aggregation wording: dialogs show it
    /// as-is on every frontend, Android included.
    fn take_modal_text(&mut self) -> Option<String> {
        let modal = self.take_batch();
        if modal.is_empty() {
            return None;
        }
        Some(
            modal
                .iter()
                .map(|line| format!("• {line}"))
                .collect::<Vec<_>>()
                .join("\n"),
        )
    }

    /// Queue one aggregated error batch for sequential display.
    /// Capped: a stuck-open dialog plus a per-tick error flood must
    /// not grow memory without bound. Drops the oldest, like the OSD
    /// pending cap (history keeps every event). Returns true when a
    /// batch was dropped.
    fn queue_modal(&mut self, detail: String) -> bool {
        let dropped = self.queue.len() >= Self::MAX_QUEUED;
        if dropped {
            self.queue.pop_front();
            self.dropped_total += 1;
        }
        self.queue.push_back(detail);
        dropped
    }

    /// Re-queue an interrupted head batch preserving display order.
    /// Overflow drops the oldest queued batch behind the head.
    fn requeue_front(&mut self, detail: String) -> bool {
        self.queue.push_front(detail);
        let mut dropped = false;
        while self.queue.len() > Self::MAX_QUEUED {
            self.queue.remove(1);
            self.dropped_total += 1;
            dropped = true;
        }
        dropped
    }

    /// Take the next batch only when no dialog is showing: frontends
    /// present one dialog at a time and ack via the modal source, so
    /// errors never stack.
    fn take_modal_batch(&mut self) -> Option<String> {
        if self.open {
            return None;
        }
        let next = self.queue.pop_front()?;
        self.open = true;
        Some(next)
    }

    fn modal_closed(&mut self) {
        self.open = false;
    }

    fn modal_pending(&self) -> usize {
        self.queue.len()
    }
}

/// Bundled inputs for [`SessionHandle::pump_osd`]: the caller-owned
/// half of the push. The channel is derived inside the pump, so an
/// empty queue skips that derivation entirely.
pub struct PushInput {
    pub policy: TransientPolicy,
    pub language: AppLanguage,
    pub now: Instant,
}

/// Outcome of [`SessionHandle::pump_osd`] for frontend flow control:
/// redraw gating and modal auto-pause.
pub struct PumpReport {
    pub changed: bool,
    pub modal_queued: bool,
}

impl OsdManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Push taking the narrow policy. Decision code here never
    /// sees the full settings surface (see [`TransientPolicy`]).
    /// Callers adapt once at the settings boundary via `From`.
    pub fn push_all_with_policy(
        &mut self,
        notifications: Vec<UiNotification>,
        policy: &TransientPolicy,
        language: AppLanguage,
        now: Instant,
        channel: FrameChannel,
    ) {
        // Dirty only on a real change: filtered rows, dropped info,
        // and queued-behind rows must not schedule a redraw.
        let mut changed = false;
        for n in notifications {
            if !enabled(n.level, policy) {
                continue;
            }
            // No composited frame and not modal-worthy: drop the row.
            if channel == FrameChannel::Unpresented && !n.kind.wants_modal() {
                continue;
            }
            if falls_back_to_modal(&n.kind, policy, channel) {
                self.modal_box
                    .push(super::notification::render_text(&n.kind, n.count, language));
                changed = true;
                continue;
            }
            let base = n.kind.text(language);
            let rgb = color(n.level, policy);
            let tag = level_tag(n.level);
            let duration = Duration::from_millis(duration_ms(n.level, policy));
            let item = OsdItem {
                row: counted_row(base.clone(), n.count, rgb, tag),
                level: n.level,
                key: base,
                count: n.count,
                duration,
                deadline: now + duration,
                group: n.kind.replace_group(),
            };
            if self.transient.route_row(item) == RouteEffect::ShowingChanged {
                changed = true;
            }
        }
        if changed {
            self.mark_changed();
        }
    }

    /// Advance the showing label. Signals via `dirty()`, which also
    /// covers `push_all` (a bool return would miss those changes).
    pub fn poll(&mut self, now: Instant) {
        if self.transient.poll(now) {
            self.mark_changed();
        }
    }

    pub fn rows(&self) -> Vec<OsdRow> {
        self.transient.rows()
    }

    /// Drain the aggregated modal batch into the sequential queue.
    /// Internal pump step for [`SessionHandle::pump_osd`]: frontends
    /// present via the modal source, never this. Overflow warns once
    /// per episode: the flag re-arms when the queue fully drains.
    pub(crate) fn pump_modal(&mut self) -> bool {
        if let Some(detail) = self.take_modal_text() {
            if self.modal_box.queue_modal(detail) && !self.modal_warned_full {
                log::warn!("error modal queue full; dropping oldest batch");
                self.modal_warned_full = true;
            }
            true
        } else {
            false
        }
    }

    fn take_modal_text(&mut self) -> Option<String> {
        self.modal_box.take_modal_text()
    }

    /// Monotonic change count. `dirty`/`take_dirty` stay for the
    /// peek-then-take protocol; new code can poll `generation`.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    fn mark_changed(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }
}

impl ModalSink for OsdManager {
    fn queue_modal(&mut self, detail: String) {
        self.modal_box.queue_modal(detail);
    }

    fn requeue_front(&mut self, detail: String) {
        self.modal_box.requeue_front(detail);
    }
}

impl ModalSource for OsdManager {
    fn take_modal_batch(&mut self) -> Option<String> {
        let next = self.modal_box.take_modal_batch();
        if next.is_some() && self.modal_box.modal_pending() == 0 {
            self.modal_warned_full = false;
        }
        next
    }

    fn modal_closed(&mut self) {
        self.modal_box.modal_closed();
        if self.modal_box.modal_pending() == 0 {
            self.modal_warned_full = false;
        }
    }

    fn modal_pending(&self) -> usize {
        self.modal_box.modal_pending()
    }
}

impl OsdManager {
    /// Total modal overflow losses (test hook for the warn-once path).
    #[cfg(test)]
    pub fn overflow_dropped(&self) -> u64 {
        self.modal_box.dropped_total
    }

    pub fn has_visible(&self) -> bool {
        self.transient.has_visible()
    }

    pub fn wakeup_deadline(&self, now: Instant) -> Option<Instant> {
        self.transient.wakeup_deadline(now)
    }

    /// The single showing row (empty when idle).
    /// Status readout split into label and value. Frontends lay out
    /// the label left and the value right-aligned in a fixed-width
    /// pill, so the digits never shift when the label appears,
    /// disappears, or changes language.
    pub fn status_parts(fps: f32, paused: bool, language: AppLanguage) -> (String, String) {
        (
            status_label(paused, language).to_string(),
            format!("{fps:.1} fps"),
        )
    }
}

/// Pause label without the fps value: cheap key for the gate
/// pre-check, so steady frames skip float formatting.
fn status_label(paused: bool, language: AppLanguage) -> &'static str {
    if paused {
        match language {
            AppLanguage::Japanese => "一時停止中",
            _ => "Paused",
        }
    } else {
        ""
    }
}

/// Minimum gap between fps-digit refreshes of the status readout.
/// Pause flips pass through immediately (events); the ticking digits
/// refresh at 2Hz so the readout stays calm and the layer is not
/// re-rasterized every frame on timer jitter.
pub const STATUS_FPS_MIN_INTERVAL: Duration = Duration::from_millis(500);

/// Gates the status value: label changes pass through, fps digits at
/// most every [`STATUS_FPS_MIN_INTERVAL`]. Frontends fold the gated
/// pair into their raster fingerprint alongside the settings.
#[derive(Debug, Default)]
pub struct StatusGate {
    last_label: Option<String>,
    last_value: Option<String>,
    last_tick: Option<Instant>,
}

impl StatusGate {
    pub fn new() -> Self {
        Self::default()
    }

    /// Gated value, computing `fresh` only on a label flip, throttle
    /// expiry, or empty cache. Steady frames skip float formatting.
    pub fn gate_or_cached(
        &mut self,
        now: Instant,
        label: String,
        fresh: impl FnOnce() -> String,
    ) -> (String, String) {
        let label_changed = self.last_label.as_deref() != Some(label.as_str());
        let tick_due = self
            .last_tick
            .is_none_or(|t| now.duration_since(t) >= STATUS_FPS_MIN_INTERVAL);
        if label_changed || tick_due || self.last_value.is_none() {
            let value = fresh();
            self.last_label = Some(label.clone());
            self.last_value = Some(value.clone());
            self.last_tick = Some(now);
            (label, value)
        } else {
            let cached = self.last_value.clone().unwrap_or_else(fresh);
            (label, cached)
        }
    }
}

/// Settings corner mapped once to the render-space anchor. The
/// rasterizer only speaks [`OsdAnchor`], so this is the single
/// place where settings vocabulary crosses into render vocabulary.
pub fn osd_anchor(position: NotificationPosition) -> OsdAnchor {
    match position {
        NotificationPosition::TopLeft => OsdAnchor::TopLeft,
        NotificationPosition::TopRight => OsdAnchor::TopRight,
        NotificationPosition::BottomLeft => OsdAnchor::BottomLeft,
        NotificationPosition::BottomRight => OsdAnchor::BottomRight,
    }
}

/// Status pill color. One constant so every frontend renders the
/// persistent readout identically; transient rows keep per-level
/// colors from [`OsdManager::push_all`].
pub const OSD_STATUS_RGB: (u8, u8, u8) = (200, 200, 200);

/// Raster-ready frame decided by [`OsdPresenter`]: rows plus the
/// gated status text, with anchors already mapped to render space.
#[derive(Debug, Clone)]
pub struct OsdPlan {
    pub rows: Vec<OsdRow>,
    pub status: Option<(String, String)>,
    pub position: OsdAnchor,
    pub status_position: OsdAnchor,
    pub width: u32,
    pub height: u32,
}

impl OsdPlan {
    /// Single construction shared by every frontend: rows plus the
    /// status pill with identical colors and corners everywhere.
    /// Takes ownership: no second clone at the renderer boundary.
    pub fn into_content(self) -> nerust_render_traits::renderer::OsdContent {
        use nerust_render_traits::renderer::{OsdContent, OsdRow, OsdStatus};
        OsdContent {
            rows: self
                .rows
                .into_iter()
                .map(|row| OsdRow {
                    tag: row.tag.to_string(),
                    text: row.text,
                    rgb: row.rgb,
                })
                .collect(),
            status: self.status.map(|(label, value)| OsdStatus {
                label,
                value,
                rgb: OSD_STATUS_RGB,
            }),
            position: self.position,
            status_position: self.status_position,
        }
    }
}

/// Narrow chrome view for [`OsdPresenter::sync`]. Only the fields
/// the layer decision reads; a new settings field cannot ripple in.
/// Adapted once per frame at the settings boundary via `From`.
#[derive(Debug, Clone, Copy)]
pub struct OsdChrome {
    pub status_enabled: bool,
    pub position: NotificationPosition,
    pub status_position: NotificationPosition,
    pub area_mode: NotificationAreaMode,
    pub status_area_mode: NotificationAreaMode,
}

impl From<&NotificationSettings> for OsdChrome {
    fn from(settings: &NotificationSettings) -> Self {
        Self {
            status_enabled: settings.status_enabled,
            position: settings.position,
            status_position: settings.status_position,
            area_mode: settings.area_mode,
            status_area_mode: settings.status_area_mode,
        }
    }
}

/// Per-frame inputs for [`OsdPresenter::sync`]. Bundled so the
/// decision signature stays narrow as inputs evolve.
pub struct OsdSyncInput {
    pub chrome: OsdChrome,
    pub language: AppLanguage,
    pub fps: f32,
    pub paused: bool,
    pub now: Instant,
    pub width: u32,
    pub height: u32,
    /// Raster scale and safe insets: they move boxes without
    /// touching settings, so the fingerprint must see them or the
    /// gate would suppress the re-render.
    pub ui_scale: f32,
    pub safe_insets: nerust_render_traits::renderer::SafeInsets,
}

/// Per-frame layer decision. `Clear` is only emitted while a layer
/// is live, so frontends call the renderer exactly as told.
#[derive(Debug, Clone)]
pub enum OsdSync {
    Skip,
    Clear,
    Render(OsdPlan),
}

/// Owns the whole sync fingerprint (transient dirty flag, status
/// gate, surface size, layer liveness) so tao/gtk/android share one
/// decision procedure instead of reimplementing it per frontend.
/// Frontends only rasterize the [`OsdPlan`] and hand it to the
/// renderer; all policy lives here and is headless-testable.
#[derive(Debug, Default)]
pub struct OsdPresenter {
    last_seen: u64,
    gate: StatusGate,
    last_status: Option<(
        String,
        String,
        NotificationPosition,
        NotificationPosition,
        NotificationAreaMode,
        NotificationAreaMode,
    )>,
    last_size: Option<(u32, u32)>,
    last_chrome: Option<(nerust_render_traits::renderer::SafeInsets, u32)>,
    active: bool,
}

impl OsdPresenter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Forget all sync state so the next pass re-pushes the layer
    /// from scratch. Call after the renderer is replaced: the new
    /// object holds no layer for the gate to skip against.
    pub fn invalidate(&mut self) {
        *self = Self::default();
    }

    /// Last display-generation this presenter synced. The pre-gate
    /// compares it without forcing a full sync.
    pub fn last_seen(&self) -> u64 {
        self.last_seen
    }

    /// Decide the layer action for this frame. The generation check
    /// replaces the old peek-then-take flag: inequality against the
    /// last synced generation means re-render. Status rides outside
    /// that flag: pause flips and settings toggles re-render
    /// immediately, fps digits refresh at 2Hz through the gate. The
    /// gate computes fresh text lazily, so steady frames skip float
    /// formatting. Untouched while disabled: no per-frame formatting,
    /// and the stale gate passes the first fresh value on re-enable.
    pub fn sync(&mut self, manager: &OsdManager, input: OsdSyncInput) -> OsdSync {
        let OsdSyncInput {
            chrome,
            language,
            fps,
            paused,
            now,
            width,
            height,
            ui_scale,
            safe_insets,
        } = input;
        let changed = manager.generation() != self.last_seen;
        self.last_seen = manager.generation();
        if width == 0 || height == 0 {
            if self.active {
                self.active = false;
                return OsdSync::Clear;
            }
            return OsdSync::Skip;
        }
        let status_key = chrome.status_enabled.then(|| {
            let (label, value) =
                self.gate
                    .gate_or_cached(now, status_label(paused, language).to_string(), || {
                        format!("{fps:.1} fps")
                    });
            (
                label,
                value,
                chrome.position,
                chrome.status_position,
                chrome.area_mode,
                chrome.status_area_mode,
            )
        });
        if status_key.is_none() {
            // Forget the throttle: re-enable must pass fresh digits.
            self.gate = StatusGate::new();
        }
        let status_changed = status_key != self.last_status;
        let raster = (safe_insets, ui_scale.to_bits());
        let chrome_changed = self.last_chrome != Some(raster);
        if !changed && !status_changed && !chrome_changed && self.last_size == Some((width, height))
        {
            return OsdSync::Skip;
        }
        self.last_size = Some((width, height));
        self.last_chrome = Some(raster);
        self.last_status = status_key.clone();
        let rows = manager.rows();
        let empty = rows.is_empty() && status_key.is_none();
        if empty {
            if self.active {
                self.active = false;
                return OsdSync::Clear;
            }
            return OsdSync::Skip;
        }
        self.active = true;
        let (label_value, position, status_position) = match status_key {
            Some((label, value, position, status_position, _, _)) => {
                (Some((label, value)), position, status_position)
            }
            None => (None, chrome.position, chrome.status_position),
        };
        OsdSync::Render(OsdPlan {
            rows,
            status: label_value,
            position: osd_anchor(position),
            status_position: osd_anchor(status_position),
            width,
            height,
        })
    }
}

/// Strip rows per edge from transient + status corners.
/// Same edge, different sides share one strip; same corner stacks two.
/// Each readout reserves only under its own area mode.
/// When both readouts share one horizontal band (both top or both
/// bottom), the transient area mode governs the edge: the status
/// picker follows it in the UI, so an overlay status never dangles
/// half inside a strip the transient reserved.
pub fn strip_rows(
    transient_mode: NotificationAreaMode,
    transient: NotificationPosition,
    status_mode: NotificationAreaMode,
    status: NotificationPosition,
    status_on: bool,
) -> (u32, u32) {
    let status_mode = if status_on && transient.same_edge(status) {
        transient_mode
    } else {
        status_mode
    };
    fn edge(position: NotificationPosition) -> bool {
        matches!(
            position,
            NotificationPosition::TopLeft | NotificationPosition::TopRight
        )
    }
    let mut top = [0u32, 0u32];
    let mut bottom = [0u32, 0u32];
    let sides = [
        (transient_mode, transient, true),
        (status_mode, status, status_on),
    ];
    for (mode, position, on) in sides {
        if !on || !matches!(mode, NotificationAreaMode::Strip) {
            continue;
        }
        let side = usize::from(matches!(
            position,
            NotificationPosition::TopRight | NotificationPosition::BottomRight
        ));
        if edge(position) {
            top[side] += 1;
        } else {
            bottom[side] += 1;
        }
    }
    (top[0].max(top[1]), bottom[0].max(bottom[1]))
}

#[cfg(test)]
mod tests {
    use super::super::notification::NotificationKind;
    use super::*;
    use crate::session::error::SlotOpFailure;
    use nerust_settings_core::model::language::AppLanguage;

    fn settings() -> NotificationSettings {
        NotificationSettings::default()
    }

    fn policy() -> TransientPolicy {
        TransientPolicy::from(&settings())
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
    fn generation_advances_only_on_visible_change() {
        use super::super::notification::NotificationKind as K;
        let mut manager = OsdManager::new();
        let now = Instant::now();
        let base = manager.generation();
        manager.push_all_with_policy(
            vec![UiNotification {
                level: NotificationLevel::Info,
                kind: K::Paused,
                count: 1,
                seq: 0,
                at: std::time::SystemTime::now(),
            }],
            &policy(),
            AppLanguage::English,
            now,
            FrameChannel::Unpresented,
        );
        // Unpresented info drops: no visible change, no generation bump.
        assert_eq!(manager.generation(), base);
        manager.push_all_with_policy(
            vec![UiNotification {
                level: NotificationLevel::Error,
                kind: K::ExternalError {
                    source: super::super::notification::ExternalSource::Bridge,
                    detail: "boom".to_string(),
                },
                count: 1,
                seq: 1,
                at: std::time::SystemTime::now(),
            }],
            &policy(),
            AppLanguage::English,
            now,
            FrameChannel::Unpresented,
        );
        assert_eq!(manager.generation(), base.wrapping_add(1));
    }

    #[test]
    fn external_error_routes_modal_without_frame() {
        use super::super::notification::{ExternalSource, NotificationKind as K};
        let kind = K::ExternalError {
            source: ExternalSource::Picker,
            detail: "picker failed".to_string(),
        };
        assert_eq!(kind.level(), NotificationLevel::Error);
        assert!(kind.wants_modal());
        assert_eq!(kind.text(AppLanguage::English), "Picker: picker failed");
        assert_eq!(kind.text(AppLanguage::Japanese), "選択: picker failed");
        let mut manager = OsdManager::new();
        manager.push_all_with_policy(
            vec![UiNotification {
                level: kind.level(),
                kind,
                count: 1,
                seq: 0,
                at: std::time::SystemTime::now(),
            }],
            &policy(),
            AppLanguage::English,
            Instant::now(),
            FrameChannel::Unpresented,
        );
        assert!(manager.rows().is_empty());
        assert!(manager.take_modal_text().is_some());
    }

    #[test]
    fn into_content_moves_rows_without_second_clone() {
        use super::super::notification::NotificationKind as K;
        let mut manager = OsdManager::new();
        let now = Instant::now();
        manager.push_all_with_policy(
            vec![UiNotification {
                level: NotificationLevel::Info,
                kind: K::Paused,
                count: 1,
                seq: 0,
                at: std::time::SystemTime::now(),
            }],
            &policy(),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        manager.poll(now);
        let mut presenter = super::OsdPresenter::new();
        let plan = match presenter.sync(
            &manager,
            super::OsdSyncInput {
                chrome: super::OsdChrome::from(&settings()),
                language: AppLanguage::English,
                fps: 60.0,
                paused: false,
                now,
                width: 320,
                height: 240,
                ui_scale: 1.0,
                safe_insets: nerust_render_traits::renderer::SafeInsets::default(),
            },
        ) {
            super::OsdSync::Render(plan) => plan,
            other => panic!("expected render, got {other:?}"),
        };
        let rows = plan.rows.len();
        assert_eq!(rows, 1);
        let content = plan.into_content();
        assert_eq!(content.rows.len(), 1);
        assert_eq!(content.rows[0].text, "Paused");
    }

    #[test]
    fn error_modal_collects_and_skips_osd() {
        let mut manager = OsdManager::new();
        let now = Instant::now();
        manager.push_all_with_policy(
            vec![UiNotification {
                level: NotificationLevel::Error,
                kind: NotificationKind::SlotFailed {
                    slot: Some(1),
                    reason: SlotOpFailure::Storage,
                },
                count: 1,
                seq: 0,
                at: std::time::SystemTime::now(),
            }],
            &policy(),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        assert!(manager.rows().is_empty());
        assert!(manager.take_modal_text().is_some());
    }

    #[test]
    fn falls_back_to_modal_covers_levels_and_channels() {
        use super::super::notification::NotificationKind as K;
        let mut modal = settings();
        modal.error_modal = true;
        let mut osd = settings();
        osd.error_modal = false;
        let error = K::SlotFailed {
            slot: Some(1),
            reason: SlotOpFailure::Storage,
        };
        let warning = K::SlotFailed {
            slot: None,
            reason: SlotOpFailure::Missing,
        };
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
            &K::Paused,
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
            &K::Paused,
            &TransientPolicy::from(&modal),
            FrameChannel::Unpresented
        ));
        assert!(!falls_back_to_modal(
            &K::Paused,
            &TransientPolicy::from(&osd),
            FrameChannel::Unpresented
        ));
    }

    #[test]
    fn inactive_core_routes_errors_and_warnings_to_modal() {
        let mut manager = OsdManager::new();
        let mut without_modal = settings();
        without_modal.error_modal = false;
        let now = Instant::now();
        for (level, kind) in [
            (
                NotificationLevel::Error,
                NotificationKind::SlotFailed {
                    slot: Some(1),
                    reason: SlotOpFailure::Storage,
                },
            ),
            (
                NotificationLevel::Warning,
                NotificationKind::SlotFailed {
                    slot: None,
                    reason: SlotOpFailure::Missing,
                },
            ),
            (NotificationLevel::Info, NotificationKind::Paused),
        ] {
            manager.push_all_with_policy(
                vec![UiNotification {
                    level,
                    kind,
                    count: 1,
                    seq: 0,
                    at: std::time::SystemTime::now(),
                }],
                &TransientPolicy::from(&without_modal),
                AppLanguage::English,
                now,
                FrameChannel::Unpresented,
            );
        }
        assert!(manager.rows().is_empty());
        // Info is dropped without a frame: only two modal lines.
        let batch = manager.take_modal_text().expect("two modal lines");
        assert_eq!(batch.lines().count(), 2);
        assert!(!batch.contains("Paused"));
    }

    #[test]
    fn info_dropped_without_frame_leaves_no_trace() {
        let mut manager = OsdManager::new();
        let now = Instant::now();
        let base = manager.generation();
        manager.push_all_with_policy(
            vec![UiNotification {
                level: NotificationLevel::Info,
                kind: NotificationKind::Paused,
                count: 1,
                seq: 0,
                at: std::time::SystemTime::now(),
            }],
            &policy(),
            AppLanguage::English,
            now,
            FrameChannel::Unpresented,
        );
        assert_eq!(manager.generation(), base);
        assert!(manager.rows().is_empty());
        assert_eq!(manager.take_modal_text(), None);
        assert_eq!(manager.modal_pending(), 0);
    }

    #[test]
    fn info_expires_after_duration() {
        let mut manager = OsdManager::new();
        let now = Instant::now();
        manager.push_all_with_policy(
            vec![UiNotification {
                level: NotificationLevel::Info,
                kind: NotificationKind::Paused,
                count: 1,
                seq: 0,
                at: std::time::SystemTime::now(),
            }],
            &policy(),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        let base = manager.generation();
        manager.poll(now);
        assert_ne!(manager.generation(), base);
        assert_eq!(manager.rows().len(), 1);
        let base = manager.generation();
        manager.poll(now + Duration::from_millis(2_001));
        assert_ne!(manager.generation(), base);
        assert!(manager.rows().is_empty());
    }

    #[test]
    fn sequential_shows_one_then_next() {
        let mut manager = OsdManager::new();
        let now = Instant::now();
        for seq in 0..2 {
            manager.push_all_with_policy(
                vec![UiNotification {
                    level: NotificationLevel::Info,
                    kind: if seq == 0 {
                        NotificationKind::Paused
                    } else {
                        NotificationKind::Resumed
                    },
                    count: 1,
                    seq,
                    at: std::time::SystemTime::now(),
                }],
                &policy(),
                AppLanguage::English,
                now,
                FrameChannel::Composited,
            );
        }
        // Showing the first; the second waits its turn.
        let base = manager.generation();
        manager.poll(now);
        assert_ne!(manager.generation(), base);
        assert_eq!(manager.rows().len(), 1);
        assert_eq!(manager.rows()[0].text, "Paused");
        // First expires after 2s; the second takes over.
        let base = manager.generation();
        manager.poll(now + Duration::from_millis(2_001));
        assert_ne!(manager.generation(), base);
        assert_eq!(manager.rows()[0].text, "Resumed");
        // Second expires; idle afterwards.
        let base = manager.generation();
        manager.poll(now + Duration::from_millis(4_002));
        assert_ne!(manager.generation(), base);
        assert!(manager.rows().is_empty());
        assert!(!manager.has_visible());
    }

    #[test]
    fn queue_caps_pending_and_coalesces_tail() {
        let mut manager = OsdManager::new();
        let now = Instant::now();
        // Five distinct pushes: showing + two queued, oldest dropped.
        for (seq, kind) in [
            NotificationKind::Paused,
            NotificationKind::Resumed,
            NotificationKind::Reset,
            NotificationKind::CreatedSlot,
            NotificationKind::Paused,
        ]
        .into_iter()
        .enumerate()
        {
            manager.push_all_with_policy(
                vec![UiNotification {
                    level: NotificationLevel::Info,
                    kind,
                    count: 1,
                    seq: seq as u64,
                    at: std::time::SystemTime::now(),
                }],
                &policy(),
                AppLanguage::English,
                now,
                FrameChannel::Composited,
            );
        }
        // Identical repeat merges with the tail (×2) instead of queuing.
        manager.push_all_with_policy(
            vec![UiNotification {
                level: NotificationLevel::Info,
                kind: NotificationKind::Paused,
                count: 1,
                seq: 99,
                at: std::time::SystemTime::now(),
            }],
            &policy(),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        let base = manager.generation();
        manager.poll(now);
        assert_ne!(manager.generation(), base);
        // Oldest pending (CreatedSlot) shows first; drops removed older.
        assert!(manager.rows()[0].text.contains("Slot"));
        // Drain fully: CreatedSlot, then merged Paused ×2, then idle.
        let mut shown = vec![manager.rows()[0].text.clone()];
        let mut t = now;
        for _ in 0..5 {
            t += Duration::from_millis(2_001);
            let base = manager.generation();
            manager.poll(t);
            if manager.generation() != base && !manager.rows().is_empty() {
                shown.push(manager.rows()[0].text.clone());
            }
        }
        assert!(manager.rows().is_empty());
        assert_eq!(shown.len(), 2);
        assert!(shown[1].contains("×2"));
    }

    #[test]
    fn error_osd_holds_five_second_minimum() {
        let mut altered = settings();
        altered.error_modal = false;
        let mut manager = OsdManager::new();
        let now = Instant::now();
        manager.push_all_with_policy(
            vec![UiNotification {
                level: NotificationLevel::Error,
                kind: NotificationKind::SlotFailed {
                    slot: None,
                    reason: SlotOpFailure::Storage,
                },
                count: 1,
                seq: 0,
                at: std::time::SystemTime::now(),
            }],
            &TransientPolicy::from(&altered),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        let base = manager.generation();
        manager.poll(now);
        assert_ne!(manager.generation(), base);
        assert_eq!(manager.rows().len(), 1);
        // Still showing past the Info duration: 5s minimum.
        let base = manager.generation();
        manager.poll(now + Duration::from_millis(2_001));
        assert_eq!(manager.generation(), base);
        assert_eq!(manager.rows().len(), 1);
        let base = manager.generation();
        manager.poll(now + Duration::from_millis(5_001));
        assert_ne!(manager.generation(), base);
        assert!(manager.rows().is_empty());
    }

    #[test]
    fn error_survives_info_flood() {
        use super::super::notification::NotificationLevel as L;
        let mut altered = settings();
        altered.error_modal = false;
        let mut manager = OsdManager::new();
        let now = Instant::now();
        let note = |kind, level| UiNotification {
            level,
            kind,
            count: 1,
            seq: 0,
            at: std::time::UNIX_EPOCH,
        };
        let error = || UiNotification {
            level: L::Error,
            kind: NotificationKind::SlotFailed {
                slot: None,
                reason: SlotOpFailure::Storage,
            },
            count: 1,
            seq: 0,
            at: std::time::UNIX_EPOCH,
        };
        // Showing Info; two Infos queued (full).
        manager.push_all_with_policy(
            vec![note(NotificationKind::Paused, L::Info)],
            &TransientPolicy::from(&altered),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        let base = manager.generation();
        manager.poll(now);
        assert_ne!(manager.generation(), base);
        manager.push_all_with_policy(
            vec![note(NotificationKind::Resumed, L::Info)],
            &TransientPolicy::from(&altered),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        manager.push_all_with_policy(
            vec![note(NotificationKind::Reset, L::Info)],
            &TransientPolicy::from(&altered),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        // An Error preempts the showing Info; the demoted label waits
        // at the queue head while the tail sheds the newest Info.
        manager.push_all_with_policy(
            vec![error()],
            &TransientPolicy::from(&altered),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        assert_eq!(manager.rows()[0].tag, "[ERROR]");
        let base = manager.generation();
        manager.poll(now + Duration::from_millis(5_001));
        assert_ne!(manager.generation(), base);
        assert!(manager.rows()[0].text.contains("Paused"));
        let base = manager.generation();
        manager.poll(now + Duration::from_millis(7_002));
        assert_ne!(manager.generation(), base);
        assert!(manager.rows()[0].text.contains("Resumed"));
    }

    #[test]
    fn error_preempts_showing_info() {
        use super::super::notification::NotificationLevel as L;
        let mut altered = settings();
        altered.error_modal = false;
        let mut manager = OsdManager::new();
        let now = Instant::now();
        manager.push_all_with_policy(
            vec![UiNotification {
                level: L::Info,
                kind: NotificationKind::Paused,
                count: 1,
                seq: 0,
                at: std::time::UNIX_EPOCH,
            }],
            &TransientPolicy::from(&altered),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        let base = manager.generation();
        manager.poll(now);
        assert_ne!(manager.generation(), base);
        assert_eq!(manager.rows()[0].tag, "[INFO]");
        manager.push_all_with_policy(
            vec![UiNotification {
                level: L::Error,
                kind: NotificationKind::SlotFailed {
                    slot: None,
                    reason: SlotOpFailure::Storage,
                },
                count: 1,
                seq: 0,
                at: std::time::UNIX_EPOCH,
            }],
            &TransientPolicy::from(&altered),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        // The Error takes the display at once; the demoted Info waits.
        assert_eq!(manager.rows()[0].tag, "[ERROR]");
        let base = manager.generation();
        manager.poll(now + Duration::from_millis(5_001));
        assert_ne!(manager.generation(), base);
        assert_eq!(manager.rows()[0].tag, "[INFO]");
    }

    #[test]
    fn warning_waits_behind_showing_info() {
        use super::super::notification::NotificationLevel as L;
        let mut manager = OsdManager::new();
        let now = Instant::now();
        manager.push_all_with_policy(
            vec![UiNotification {
                level: L::Info,
                kind: NotificationKind::Paused,
                count: 1,
                seq: 0,
                at: std::time::UNIX_EPOCH,
            }],
            &policy(),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        let base = manager.generation();
        manager.poll(now);
        assert_ne!(manager.generation(), base);
        manager.push_all_with_policy(
            vec![UiNotification {
                level: L::Warning,
                kind: NotificationKind::SlotFailed {
                    slot: None,
                    reason: SlotOpFailure::Missing,
                },
                count: 1,
                seq: 0,
                at: std::time::UNIX_EPOCH,
            }],
            &policy(),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        // Only Errors preempt; the Warning waits for expiry.
        assert_eq!(manager.rows()[0].tag, "[INFO]");
        let base = manager.generation();
        manager.poll(now + Duration::from_millis(2_001));
        assert_ne!(manager.generation(), base);
        assert_eq!(manager.rows()[0].tag, "[WARN]");
    }

    #[test]
    fn queue_overflow_drops_lowest_severity_first() {
        use super::super::notification::NotificationLevel as L;
        let mut altered = settings();
        altered.error_modal = false;
        let mut manager = OsdManager::new();
        let now = Instant::now();
        let error = |slot| UiNotification {
            level: L::Error,
            kind: NotificationKind::SlotFailed {
                slot,
                reason: SlotOpFailure::Storage,
            },
            count: 1,
            seq: 0,
            at: std::time::UNIX_EPOCH,
        };
        let warning = UiNotification {
            level: L::Warning,
            kind: NotificationKind::SlotFailed {
                slot: None,
                reason: SlotOpFailure::Missing,
            },
            count: 1,
            seq: 0,
            at: std::time::UNIX_EPOCH,
        };
        let info = |kind| UiNotification {
            level: L::Info,
            kind,
            count: 1,
            seq: 0,
            at: std::time::UNIX_EPOCH,
        };
        // Showing Error (no preempt against itself); queue Warning + Info.
        manager.push_all_with_policy(
            vec![error(Some(9))],
            &TransientPolicy::from(&altered),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        let base = manager.generation();
        manager.poll(now);
        assert_ne!(manager.generation(), base);
        manager.push_all_with_policy(
            vec![warning],
            &TransientPolicy::from(&altered),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        manager.push_all_with_policy(
            vec![info(NotificationKind::Paused)],
            &TransientPolicy::from(&altered),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        // A second Info overflows: the oldest Info drops, Warning and
        // Error survive.
        manager.push_all_with_policy(
            vec![info(NotificationKind::Reset)],
            &TransientPolicy::from(&altered),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        let base = manager.generation();
        manager.poll(now + Duration::from_millis(5_001));
        assert_ne!(manager.generation(), base);
        assert_eq!(manager.rows()[0].tag, "[WARN]");
        let base = manager.generation();
        manager.poll(now + Duration::from_millis(10_002));
        assert_ne!(manager.generation(), base);
        assert!(manager.rows()[0].text.contains("Reset"));
        assert_eq!(manager.rows()[0].tag, "[INFO]");
    }

    #[test]
    fn error_queue_bound_drops_oldest_error() {
        use super::super::notification::NotificationLevel as L;
        let mut altered = settings();
        altered.error_modal = false;
        let mut manager = OsdManager::new();
        let now = Instant::now();
        let error = |slot| UiNotification {
            level: L::Error,
            kind: NotificationKind::SlotFailed {
                slot,
                reason: SlotOpFailure::Storage,
            },
            count: 1,
            seq: 0,
            at: std::time::UNIX_EPOCH,
        };
        // Three Errors, nothing showing: the oldest drops (FIFO bound).
        manager.push_all_with_policy(
            vec![error(Some(1))],
            &TransientPolicy::from(&altered),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        manager.push_all_with_policy(
            vec![error(Some(2))],
            &TransientPolicy::from(&altered),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        manager.push_all_with_policy(
            vec![error(Some(3))],
            &TransientPolicy::from(&altered),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        let base = manager.generation();
        manager.poll(now);
        assert_ne!(manager.generation(), base);
        assert!(manager.rows()[0].text.contains("Slot 2"));
        let base = manager.generation();
        manager.poll(now + Duration::from_millis(5_001));
        assert_ne!(manager.generation(), base);
        assert!(manager.rows()[0].text.contains("Slot 3"));
    }

    #[test]
    fn demote_overflow_sheds_tail_info_first() {
        use super::super::notification::NotificationLevel as L;
        let mut altered = settings();
        altered.error_modal = false;
        let mut manager = OsdManager::new();
        let now = Instant::now();
        let error = |slot| UiNotification {
            level: L::Error,
            kind: NotificationKind::SlotFailed {
                slot,
                reason: SlotOpFailure::Storage,
            },
            count: 1,
            seq: 0,
            at: std::time::UNIX_EPOCH,
        };
        let info = |kind| UiNotification {
            level: L::Info,
            kind,
            count: 1,
            seq: 0,
            at: std::time::UNIX_EPOCH,
        };
        // Showing Info with a Warning and an Info queued (full).
        // Errors always preempt a showing Info, so a full Error
        // queue behind it is unreachable; the demote victim here is
        // the tail Info.
        manager.push_all_with_policy(
            vec![info(NotificationKind::Paused)],
            &TransientPolicy::from(&altered),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        let base = manager.generation();
        manager.poll(now);
        assert_ne!(manager.generation(), base);
        manager.push_all_with_policy(
            vec![UiNotification {
                level: L::Warning,
                kind: NotificationKind::SlotFailed {
                    slot: None,
                    reason: SlotOpFailure::Missing,
                },
                count: 1,
                seq: 0,
                at: std::time::UNIX_EPOCH,
            }],
            &TransientPolicy::from(&altered),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        manager.push_all_with_policy(
            vec![info(NotificationKind::Reset)],
            &TransientPolicy::from(&altered),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        // The Error preempts; the demote insert overflows and the
        // tail Info drops so the demoted head survives.
        manager.push_all_with_policy(
            vec![error(Some(1))],
            &TransientPolicy::from(&altered),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        assert_eq!(manager.rows()[0].tag, "[ERROR]");
        let base = manager.generation();
        manager.poll(now + Duration::from_millis(5_000));
        assert_ne!(manager.generation(), base);
        assert!(manager.rows()[0].text.contains("Paused"));
        let base = manager.generation();
        manager.poll(now + Duration::from_millis(7_000));
        assert_ne!(manager.generation(), base);
        assert_eq!(manager.rows()[0].tag, "[WARN]");
        let base = manager.generation();
        manager.poll(now + Duration::from_millis(12_000));
        assert_ne!(manager.generation(), base);
        assert!(manager.rows().is_empty());
    }

    #[test]
    fn selected_slot_latest_wins() {
        use super::super::notification::NotificationLevel as L;
        let mut manager = OsdManager::new();
        let now = Instant::now();
        let note = |slot| UiNotification {
            level: L::Info,
            kind: NotificationKind::SelectedSlot(slot),
            count: 1,
            seq: 0,
            at: std::time::UNIX_EPOCH,
        };
        manager.push_all_with_policy(
            vec![note(3)],
            &policy(),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        manager.push_all_with_policy(
            vec![note(5)],
            &policy(),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        // One row only, showing the latest selection.
        let base = manager.generation();
        manager.poll(now);
        assert_ne!(manager.generation(), base);
        assert!(manager.rows()[0].text.contains("slot 5"));
        assert!(!manager.rows()[0].text.contains("slot 3"));
        let base = manager.generation();
        manager.poll(now + Duration::from_millis(2_001));
        assert_ne!(manager.generation(), base);
        assert!(manager.rows().is_empty());
    }

    #[test]
    fn selected_slot_updates_showing_row() {
        use super::super::notification::NotificationLevel as L;
        let mut manager = OsdManager::new();
        let now = Instant::now();
        let note = |slot| UiNotification {
            level: L::Info,
            kind: NotificationKind::SelectedSlot(slot),
            count: 1,
            seq: 0,
            at: std::time::UNIX_EPOCH,
        };
        manager.push_all_with_policy(
            vec![note(3)],
            &policy(),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        let base = manager.generation();
        manager.poll(now);
        assert_ne!(manager.generation(), base);
        assert!(manager.rows()[0].text.contains("slot 3"));
        // A newer selection rewrites the showing row instead of queuing.
        manager.push_all_with_policy(
            vec![note(7)],
            &policy(),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        assert!(manager.rows()[0].text.contains("slot 7"));
        let base = manager.generation();
        manager.poll(now + Duration::from_millis(2_001));
        assert_ne!(manager.generation(), base);
        assert!(manager.rows().is_empty());
    }

    #[test]
    fn strip_rows_share_edges() {
        use NotificationPosition as P;
        assert_eq!(
            strip_rows(
                NotificationAreaMode::Strip,
                P::BottomLeft,
                NotificationAreaMode::Strip,
                P::BottomRight,
                true
            ),
            (0, 1)
        );
        assert_eq!(
            strip_rows(
                NotificationAreaMode::Strip,
                P::BottomLeft,
                NotificationAreaMode::Strip,
                P::BottomLeft,
                true
            ),
            (0, 2)
        );
        assert_eq!(
            strip_rows(
                NotificationAreaMode::Overlay,
                P::BottomLeft,
                NotificationAreaMode::Overlay,
                P::TopLeft,
                true
            ),
            (0, 0)
        );
        // Independent modes: overlay status reserves nothing.
        assert_eq!(
            strip_rows(
                NotificationAreaMode::Strip,
                P::BottomLeft,
                NotificationAreaMode::Overlay,
                P::TopLeft,
                true
            ),
            (0, 1)
        );
        // ... and an overlay transient leaves a strip status alone.
        assert_eq!(
            strip_rows(
                NotificationAreaMode::Overlay,
                P::BottomLeft,
                NotificationAreaMode::Strip,
                P::TopLeft,
                true
            ),
            (1, 0)
        );
    }

    #[test]
    fn level_tag_and_name_stay_distinct() {
        use super::super::notification::NotificationLevel as L;
        assert_eq!(super::level_tag(L::Warning), "[WARN]");
        assert_eq!(super::level_name(L::Warning), "Warning");
        assert_eq!(super::level_name(L::Error), "Error");
    }

    #[test]
    fn history_datetime_is_local_wall_clock() {
        // Only this test writes TZ; the rest assert TZ-free
        // structure, so parallel execution stays sound.
        // SAFETY: single-process test env, no other TZ writer.
        unsafe { std::env::set_var("TZ", "UTC0") };
        let epoch = std::time::UNIX_EPOCH;
        assert_eq!(format_datetime(epoch), "1970-01-01 00:00:00");
        assert_eq!(format_time(epoch), "00:00:00");
        let known = epoch + std::time::Duration::from_secs(1_769_990_400);
        assert_eq!(format_datetime(known), "2026-02-02 00:00:00");
    }

    #[test]
    fn history_datetime_keeps_fixed_shape_and_order() {
        let epoch = std::time::UNIX_EPOCH;
        let rendered = format_datetime(epoch);
        assert_eq!(rendered.len(), 19);
        assert_eq!(&rendered[4..5], "-");
        assert_eq!(&rendered[10..11], " ");
        assert_eq!(&rendered[13..14], ":");
        // A full day always advances wall-clock, in any zone.
        let next = format_datetime(epoch + std::time::Duration::from_secs(86_400));
        assert!(rendered < next);
    }

    #[test]
    fn history_lines_batch_matches_single_formatting() {
        use super::super::notification::{NotificationKind, UiNotification};
        // Injecting SystemTime directly is impossible here, so the
        // batch is checked structurally: one row per event, ×N suffix
        // preserved, level carried through.
        let events = [
            UiNotification {
                level: super::super::notification::NotificationLevel::Info,
                kind: NotificationKind::Paused,
                count: 1,
                seq: 0,
                at: std::time::UNIX_EPOCH,
            },
            UiNotification {
                level: super::super::notification::NotificationLevel::Error,
                kind: NotificationKind::CommandFailed("Save"),
                count: 3,
                seq: 1,
                at: std::time::UNIX_EPOCH,
            },
        ];
        let rows = history_lines(
            events.iter(),
            nerust_settings_core::model::language::AppLanguage::English,
        );
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].datetime, format_datetime(std::time::UNIX_EPOCH));
        assert_eq!(rows[0].text, "Paused");
        assert!(rows[1].text.ends_with(" ×3"));
        assert_eq!(rows[1].count, 3);
    }

    #[test]
    fn modal_queue_caps_and_drops_oldest() {
        let mut manager = OsdManager::new();
        for i in 0..10 {
            manager.queue_modal(format!("batch {i}"));
        }
        assert_eq!(manager.modal_pending(), 8);
        // Oldest two dropped; order preserved after that.
        assert_eq!(manager.take_modal_batch().as_deref(), Some("batch 2"));
        manager.modal_closed();
    }

    #[test]
    fn level_names_stay_distinct_per_surface() {
        use super::super::notification::NotificationLevel as L;
        assert_eq!(super::level_tag(L::Info), "[INFO]");
        assert_eq!(super::level_name(L::Info), "Info");
        assert_eq!(super::level_label(L::Info, AppLanguage::English), "Info");
        assert_eq!(
            super::level_label(L::Warning, AppLanguage::Japanese),
            "警告"
        );
        assert_eq!(super::level_label(L::Error, AppLanguage::English), "Error");
        assert_eq!(
            super::level_label(L::Error, AppLanguage::Japanese),
            "エラー"
        );
    }

    #[test]
    fn status_text_names_pause_state() {
        assert_eq!(
            OsdManager::status_parts(60.0, true, AppLanguage::English),
            ("Paused".to_string(), "60.0 fps".to_string())
        );
        assert_eq!(
            OsdManager::status_parts(60.0, true, AppLanguage::Japanese),
            ("一時停止中".to_string(), "60.0 fps".to_string())
        );
        assert_eq!(
            OsdManager::status_parts(59.9, false, AppLanguage::English),
            (String::new(), "59.9 fps".to_string())
        );
    }

    #[test]
    fn status_gate_throttles_digits_but_not_labels() {
        use super::{STATUS_FPS_MIN_INTERVAL, StatusGate};
        let mut gate = StatusGate::new();
        let now = Instant::now();
        // First sample passes.
        assert_eq!(
            gate.gate_or_cached(now, String::new(), || "60.1 fps".to_string()),
            (String::new(), "60.1 fps".to_string())
        );
        // Jitter within the interval holds the old digits.
        assert_eq!(
            gate.gate_or_cached(now, String::new(), || "60.2 fps".to_string()),
            (String::new(), "60.1 fps".to_string())
        );
        // After the interval the fresh digits pass.
        assert_eq!(
            gate.gate_or_cached(now + STATUS_FPS_MIN_INTERVAL, String::new(), || {
                "60.2 fps".to_string()
            }),
            (String::new(), "60.2 fps".to_string())
        );
        // A pause flip passes immediately even inside the interval.
        assert_eq!(
            gate.gate_or_cached(now + STATUS_FPS_MIN_INTERVAL, "Paused".to_string(), || {
                "60.3 fps".to_string()
            }),
            ("Paused".to_string(), "60.3 fps".to_string())
        );
    }

    #[test]
    fn interleaved_pause_resume_queues_sequentially() {
        use super::super::notification::NotificationKind as K;
        use super::super::notification::NotificationLevel as L;
        let mut manager = OsdManager::new();
        let t0 = Instant::now();
        let at = |ms| t0 + Duration::from_millis(ms);
        let note = |kind| UiNotification {
            level: L::Info,
            kind,
            count: 1,
            seq: 0,
            at: std::time::UNIX_EPOCH,
        };
        macro_rules! shown {
            () => {
                manager.rows().first().map(|row| row.text.clone())
            };
        }
        // Pause, +0.5s resume, +0.5s pause, +0.5s resume (2s slots).
        manager.push_all_with_policy(
            vec![note(K::Paused)],
            &policy(),
            AppLanguage::English,
            at(0),
            FrameChannel::Composited,
        );
        let base = manager.generation();
        manager.poll(at(0));
        assert_ne!(manager.generation(), base);
        assert_eq!(shown!().as_deref(), Some("Paused"));
        manager.push_all_with_policy(
            vec![note(K::Resumed)],
            &policy(),
            AppLanguage::English,
            at(500),
            FrameChannel::Composited,
        );
        let base = manager.generation();
        manager.poll(at(500));
        assert_eq!(manager.generation(), base);
        // Interleaved: must queue behind, never merge into the showing label.
        manager.push_all_with_policy(
            vec![note(K::Paused)],
            &policy(),
            AppLanguage::English,
            at(1000),
            FrameChannel::Composited,
        );
        assert_eq!(shown!().as_deref(), Some("Paused"));
        manager.push_all_with_policy(
            vec![note(K::Resumed)],
            &policy(),
            AppLanguage::English,
            at(1500),
            FrameChannel::Composited,
        );
        // Full 2s slots in arrival order; promoted labels measure
        // from promotion, not from push time. The cap drops the
        // oldest pending (the first resume), so the legs read
        // pause, pause, resume with no accumulation.
        let base = manager.generation();
        manager.poll(at(2000));
        assert_ne!(manager.generation(), base);
        assert_eq!(shown!().as_deref(), Some("Paused"));
        let base = manager.generation();
        manager.poll(at(3500));
        assert_eq!(manager.generation(), base);
        let base = manager.generation();
        manager.poll(at(4000));
        assert_ne!(manager.generation(), base);
        assert_eq!(shown!().as_deref(), Some("Resumed"));
        let base = manager.generation();
        manager.poll(at(5500));
        assert_eq!(manager.generation(), base);
        let base = manager.generation();
        manager.poll(at(6000));
        assert_ne!(manager.generation(), base);
        assert!(manager.rows().is_empty());
        let base = manager.generation();
        manager.poll(at(8000));
        assert_eq!(manager.generation(), base);
    }

    #[test]
    fn identical_pushes_merge_and_reset_the_timer() {
        use super::super::notification::NotificationLevel as L;
        let mut manager = OsdManager::new();
        let now = Instant::now();
        let note = |count| UiNotification {
            level: L::Info,
            kind: NotificationKind::Paused,
            count,
            seq: 0,
            at: std::time::UNIX_EPOCH,
        };
        manager.push_all_with_policy(
            vec![note(1)],
            &policy(),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        manager.push_all_with_policy(
            vec![note(1)],
            &policy(),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        let base = manager.generation();
        manager.poll(now);
        assert_ne!(manager.generation(), base);
        assert_eq!(manager.rows().len(), 1);
        assert!(manager.rows()[0].text.ends_with(" ×2"));
        assert!(manager.has_visible());
        // Still showing before the reset deadline; gone after it.
        let base = manager.generation();
        manager.poll(now + std::time::Duration::from_secs(1));
        assert_eq!(manager.generation(), base);
        let base = manager.generation();
        manager.poll(now + std::time::Duration::from_secs(61));
        assert_ne!(manager.generation(), base);
        assert!(!manager.has_visible());
    }

    #[test]
    fn wakeup_deadline_distinguishes_idle_and_queued() {
        use super::super::notification::NotificationLevel as L;
        let mut manager = OsdManager::new();
        let now = Instant::now();
        assert_eq!(manager.wakeup_deadline(now), None);
        manager.push_all_with_policy(
            vec![UiNotification {
                level: L::Info,
                kind: NotificationKind::Paused,
                count: 1,
                seq: 0,
                at: std::time::UNIX_EPOCH,
            }],
            &policy(),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        // Pending with nothing showing: wake immediately on our clock.
        assert_eq!(manager.wakeup_deadline(now), Some(now));
        // Promoted on the next poll; deadline lands on the duration.
        let base = manager.generation();
        manager.poll(now);
        assert_ne!(manager.generation(), base);
        assert!(manager.wakeup_deadline(now).is_some());
    }

    #[test]
    fn queued_behind_leaves_clean_until_promotion() {
        use super::super::notification::NotificationLevel as L;
        let mut manager = OsdManager::new();
        let now = Instant::now();
        let note = |kind| UiNotification {
            level: L::Info,
            kind,
            count: 1,
            seq: 0,
            at: std::time::UNIX_EPOCH,
        };
        manager.push_all_with_policy(
            vec![note(NotificationKind::Paused)],
            &policy(),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        let base = manager.generation();
        manager.poll(now);
        assert_ne!(manager.generation(), base);
        // Queued behind the showing row: no dirty, deadline armed.
        let base = manager.generation();
        manager.push_all_with_policy(
            vec![note(NotificationKind::Resumed)],
            &policy(),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        assert_eq!(manager.generation(), base);
        assert!(manager.wakeup_deadline(now).is_some());
        // Expiry promotes the queued row and dirties.
        manager.poll(now + Duration::from_millis(2_001));
        assert_ne!(manager.generation(), base);
        assert_eq!(manager.rows()[0].text, "Resumed");
    }

    #[test]
    fn merged_showing_row_dirties() {
        use super::super::notification::NotificationLevel as L;
        let mut manager = OsdManager::new();
        let now = Instant::now();
        let note = UiNotification {
            level: L::Info,
            kind: NotificationKind::Paused,
            count: 1,
            seq: 0,
            at: std::time::UNIX_EPOCH,
        };
        manager.push_all_with_policy(
            vec![note.clone()],
            &policy(),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        let base = manager.generation();
        manager.poll(now);
        assert_ne!(manager.generation(), base);
        // Consecutive duplicate merges into the showing row: visible.
        let base = manager.generation();
        manager.push_all_with_policy(
            vec![note],
            &policy(),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        assert_ne!(manager.generation(), base);
        assert!(manager.rows()[0].text.ends_with(" ×2"));
    }

    #[test]
    fn modal_overflow_counts_losses() {
        use super::{ModalSink, ModalSource};
        let mut manager = OsdManager::new();
        for i in 0..9 {
            manager.queue_modal(format!("batch {i}"));
        }
        // Nine batches into a cap of eight: one loss recorded.
        assert_eq!(manager.overflow_dropped(), 1);
        assert_eq!(manager.modal_pending(), 8);
        // Drain every batch through take and close.
        while manager.modal_pending() > 0 {
            manager.take_modal_batch().expect("queued batch");
            manager.modal_closed();
        }
        // A fresh overflow episode counts again.
        for i in 0..9 {
            manager.queue_modal(format!("again {i}"));
        }
        assert_eq!(manager.overflow_dropped(), 2);
    }

    #[test]
    fn modal_requeue_overflow_counts_losses() {
        use super::{ModalSink, ModalSource};
        let mut manager = OsdManager::new();
        for i in 0..8 {
            manager.queue_modal(format!("batch {i}"));
        }
        let taken = manager.take_modal_batch().expect("head batch");
        // Refill while the head is out, then suspend-requeue it back:
        // both overflow legs record a loss.
        for i in 0..2 {
            manager.queue_modal(format!("refill {i}"));
        }
        manager.requeue_front(taken);
        assert_eq!(manager.overflow_dropped(), 2);
        assert_eq!(manager.modal_pending(), 8);
    }

    #[test]
    fn gate_or_cached_computes_fresh_only_on_change() {
        use super::StatusGate;
        let mut gate = StatusGate::new();
        let now = Instant::now();
        // Fresh gate: fresh computed once.
        let mut fresh_calls = 0;
        let (label, value) = gate.gate_or_cached(now, "Paused".to_string(), || {
            fresh_calls += 1;
            "60.0 fps".to_string()
        });
        assert_eq!((label.as_str(), value.as_str()), ("Paused", "60.0 fps"));
        assert_eq!(fresh_calls, 1);
        // Steady: fresh never runs again.
        let (_, steady) = gate.gate_or_cached(now, "Paused".to_string(), || {
            fresh_calls += 1;
            "60.1 fps".to_string()
        });
        assert_eq!(steady, "60.0 fps");
        assert_eq!(fresh_calls, 1);
    }

    #[test]
    fn strip_rows_counts_edges_and_sides() {
        use nerust_settings_core::model::local::{
            NotificationAreaMode as M, NotificationPosition as P,
        };
        assert_eq!(
            super::strip_rows(M::Overlay, P::TopLeft, M::Overlay, P::BottomLeft, true),
            (0, 0)
        );
        assert_eq!(
            super::strip_rows(M::Strip, P::TopLeft, M::Strip, P::BottomLeft, true),
            (1, 1)
        );
        assert_eq!(
            super::strip_rows(M::Strip, P::TopRight, M::Strip, P::BottomRight, true),
            (1, 1)
        );
        // Same corner stacks two rows on one side.
        assert_eq!(
            super::strip_rows(M::Strip, P::TopLeft, M::Strip, P::TopLeft, true),
            (2, 0)
        );
        // Status off: transient row only.
        assert_eq!(
            super::strip_rows(M::Strip, P::BottomLeft, M::Strip, P::TopLeft, false),
            (0, 1)
        );
        // Same band: the transient area mode governs the edge, so the
        // status picker cannot dangle an overlay inside a strip.
        assert_eq!(
            super::strip_rows(M::Strip, P::BottomLeft, M::Overlay, P::BottomLeft, true),
            (0, 2)
        );
        assert_eq!(
            super::strip_rows(M::Overlay, P::BottomLeft, M::Strip, P::BottomLeft, true),
            (0, 0)
        );
        assert_eq!(
            super::strip_rows(M::Strip, P::BottomLeft, M::Overlay, P::BottomRight, true),
            (0, 1)
        );
        assert_eq!(
            super::strip_rows(M::Overlay, P::BottomLeft, M::Strip, P::BottomRight, true),
            (0, 0)
        );
    }

    #[test]
    fn modal_queue_shows_one_at_a_time() {
        let mut manager = OsdManager::new();
        manager.queue_modal("first".to_string());
        manager.queue_modal("second".to_string());
        assert_eq!(manager.take_modal_batch().as_deref(), Some("first"));
        // Open dialog blocks the next batch.
        assert_eq!(manager.take_modal_batch(), None);
        assert_eq!(manager.modal_pending(), 1);
        manager.modal_closed();
        assert_eq!(manager.take_modal_batch().as_deref(), Some("second"));
        manager.modal_closed();
        assert_eq!(manager.take_modal_batch(), None);
    }

    #[test]
    fn anchor_mapping_covers_all_corners() {
        use nerust_render_traits::renderer::OsdAnchor as A;
        use nerust_settings_core::model::local::NotificationPosition as P;
        assert_eq!(super::osd_anchor(P::TopLeft), A::TopLeft);
        assert_eq!(super::osd_anchor(P::TopRight), A::TopRight);
        assert_eq!(super::osd_anchor(P::BottomLeft), A::BottomLeft);
        assert_eq!(super::osd_anchor(P::BottomRight), A::BottomRight);
    }

    fn presenter_settings() -> nerust_settings_core::model::local::NotificationSettings {
        nerust_settings_core::model::local::NotificationSettings::default()
    }

    #[test]
    fn presenter_skips_when_idle_and_clears_zero_size() {
        use super::{OsdPresenter, OsdSync};
        use std::time::Instant;
        let manager = OsdManager::new();
        let mut presenter = OsdPresenter::new();
        let settings = presenter_settings();
        // Idle, no status: nothing to draw, nothing live.
        assert!(matches!(
            presenter.sync(
                &manager,
                super::OsdSyncInput {
                    chrome: super::OsdChrome::from(&settings),
                    language: AppLanguage::English,
                    fps: 60.0,
                    paused: false,
                    now: Instant::now(),
                    width: 960,
                    height: 720,
                    ui_scale: 1.0,
                    safe_insets: nerust_render_traits::renderer::SafeInsets::default(),
                }
            ),
            OsdSync::Skip
        ));
        // Zero-size while idle stays skipped.
        assert!(matches!(
            presenter.sync(
                &manager,
                super::OsdSyncInput {
                    chrome: super::OsdChrome::from(&settings),
                    language: AppLanguage::English,
                    fps: 60.0,
                    paused: false,
                    now: Instant::now(),
                    width: 0,
                    height: 0,
                    ui_scale: 1.0,
                    safe_insets: nerust_render_traits::renderer::SafeInsets::default(),
                }
            ),
            OsdSync::Skip
        ));
    }

    #[test]
    fn presenter_renders_status_then_skips_and_clears() {
        use super::{OsdPresenter, OsdSync};
        use std::time::Instant;
        let manager = OsdManager::new();
        let mut presenter = OsdPresenter::new();
        let mut settings = presenter_settings();
        settings.status_enabled = true;
        let now = Instant::now();
        // First frame renders the status readout.
        let plan = match presenter.sync(
            &manager,
            super::OsdSyncInput {
                chrome: super::OsdChrome::from(&settings),
                language: AppLanguage::English,
                fps: 60.0,
                paused: false,
                now,
                width: 960,
                height: 720,
                ui_scale: 1.0,
                safe_insets: nerust_render_traits::renderer::SafeInsets::default(),
            },
        ) {
            OsdSync::Render(plan) => plan,
            other => panic!("expected render, got {other:?}"),
        };
        assert_eq!(
            plan.status.as_ref().map(|(l, v)| (l.as_str(), v.as_str())),
            Some(("", "60.0 fps"))
        );
        assert_eq!(plan.width, 960);
        // Same inputs: fps jitter inside the gate holds the digits.
        assert!(matches!(
            presenter.sync(
                &manager,
                super::OsdSyncInput {
                    chrome: super::OsdChrome::from(&settings),
                    language: AppLanguage::English,
                    fps: 60.2,
                    paused: false,
                    now,
                    width: 960,
                    height: 720,
                    ui_scale: 1.0,
                    safe_insets: nerust_render_traits::renderer::SafeInsets::default(),
                }
            ),
            OsdSync::Skip
        ));
        // Pause flips pass through immediately.
        assert!(matches!(
            presenter.sync(
                &manager,
                super::OsdSyncInput {
                    chrome: super::OsdChrome::from(&settings),
                    language: AppLanguage::English,
                    fps: 60.2,
                    paused: true,
                    now,
                    width: 960,
                    height: 720,
                    ui_scale: 1.0,
                    safe_insets: nerust_render_traits::renderer::SafeInsets::default(),
                }
            ),
            OsdSync::Render(_)
        ));
        // Disabling status with no rows clears the live layer.
        settings.status_enabled = false;
        assert!(matches!(
            presenter.sync(
                &manager,
                super::OsdSyncInput {
                    chrome: super::OsdChrome::from(&settings),
                    language: AppLanguage::English,
                    fps: 60.2,
                    paused: false,
                    now,
                    width: 960,
                    height: 720,
                    ui_scale: 1.0,
                    safe_insets: nerust_render_traits::renderer::SafeInsets::default(),
                }
            ),
            OsdSync::Clear
        ));
        // Clearing again stays skipped: nothing live.
        assert!(matches!(
            presenter.sync(
                &manager,
                super::OsdSyncInput {
                    chrome: super::OsdChrome::from(&settings),
                    language: AppLanguage::English,
                    fps: 60.2,
                    paused: false,
                    now,
                    width: 960,
                    height: 720,
                    ui_scale: 1.0,
                    safe_insets: nerust_render_traits::renderer::SafeInsets::default(),
                }
            ),
            OsdSync::Skip
        ));
    }

    fn sync_input(
        chrome: super::OsdChrome,
        now: std::time::Instant,
        width: u32,
        height: u32,
        ui_scale: f32,
        safe_insets: nerust_render_traits::renderer::SafeInsets,
    ) -> super::OsdSyncInput {
        super::OsdSyncInput {
            chrome,
            language: AppLanguage::English,
            fps: 60.0,
            paused: false,
            now,
            width,
            height,
            ui_scale,
            safe_insets,
        }
    }

    fn chrome_of(settings: &NotificationSettings) -> super::OsdChrome {
        super::OsdChrome::from(settings)
    }

    #[test]
    fn presenter_rerenders_on_chrome_change() {
        use super::{OsdPresenter, OsdSync};
        use nerust_render_traits::renderer::SafeInsets;
        let manager = OsdManager::new();
        let mut presenter = OsdPresenter::new();
        let mut settings = presenter_settings();
        settings.status_enabled = true;
        let now = std::time::Instant::now();
        let plain = SafeInsets::default();
        let inset = SafeInsets {
            top: 40,
            right: 24,
            bottom: 40,
            left: 24,
        };
        // First frame renders the status readout.
        assert!(matches!(
            presenter.sync(
                &manager,
                sync_input(chrome_of(&settings), now, 960, 720, 1.0, plain)
            ),
            OsdSync::Render(_)
        ));
        // Steady inputs skip.
        assert!(matches!(
            presenter.sync(
                &manager,
                sync_input(chrome_of(&settings), now, 960, 720, 1.0, plain)
            ),
            OsdSync::Skip
        ));
        // Insets move the boxes: re-render without settings churn.
        assert!(matches!(
            presenter.sync(
                &manager,
                sync_input(chrome_of(&settings), now, 960, 720, 1.0, inset)
            ),
            OsdSync::Render(_)
        ));
        // Scale change re-renders too.
        assert!(matches!(
            presenter.sync(
                &manager,
                sync_input(chrome_of(&settings), now, 960, 720, 0.5, inset)
            ),
            OsdSync::Render(_)
        ));
        // Steady again skips.
        assert!(matches!(
            presenter.sync(
                &manager,
                sync_input(chrome_of(&settings), now, 960, 720, 0.5, inset)
            ),
            OsdSync::Skip
        ));
    }

    #[test]
    fn presenter_invalidate_forces_full_repush() {
        use super::{OsdPresenter, OsdSync};
        use nerust_render_traits::renderer::SafeInsets;
        let manager = OsdManager::new();
        let mut presenter = OsdPresenter::new();
        let mut settings = presenter_settings();
        settings.status_enabled = true;
        let now = std::time::Instant::now();
        let input = || {
            sync_input(
                chrome_of(&settings),
                now,
                960,
                720,
                1.0,
                SafeInsets::default(),
            )
        };
        assert!(matches!(
            presenter.sync(&manager, input()),
            OsdSync::Render(_)
        ));
        assert!(matches!(presenter.sync(&manager, input()), OsdSync::Skip));
        presenter.invalidate();
        assert!(matches!(
            presenter.sync(&manager, input()),
            OsdSync::Render(_)
        ));
    }

    #[test]
    fn peek_does_not_starve_sync() {
        use super::super::notification::NotificationKind;
        use super::{FrameChannel, OsdPresenter, OsdSync};
        let mut manager = OsdManager::new();
        let mut presenter = OsdPresenter::new();
        let settings = presenter_settings();
        let now = std::time::Instant::now();
        manager.push_all_with_policy(
            vec![UiNotification {
                level: NotificationLevel::Info,
                kind: NotificationKind::Paused,
                count: 1,
                seq: 0,
                at: std::time::SystemTime::now(),
            }],
            &TransientPolicy::from(&settings),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        let base = manager.generation();
        manager.poll(now);
        assert_ne!(manager.generation(), base);
        // Frontend two-phase: redraw poll peeks, sync takes same pass.
        assert_ne!(manager.generation(), base);
        assert_ne!(manager.generation(), base);
        assert!(matches!(
            presenter.sync(
                &manager,
                sync_input(chrome_of(&settings), now, 960, 720, 1.0, Default::default())
            ),
            OsdSync::Render(_)
        ));
        assert!(matches!(
            presenter.sync(
                &manager,
                sync_input(chrome_of(&settings), now, 960, 720, 1.0, Default::default())
            ),
            OsdSync::Skip
        ));
    }

    #[test]
    fn peek_does_not_starve_expiry_clear() {
        use super::super::notification::NotificationKind;
        use super::{FrameChannel, OsdPresenter, OsdSync};
        let mut manager = OsdManager::new();
        let mut presenter = OsdPresenter::new();
        let settings = presenter_settings();
        let now = std::time::Instant::now();
        manager.push_all_with_policy(
            vec![UiNotification {
                level: NotificationLevel::Info,
                kind: NotificationKind::Paused,
                count: 1,
                seq: 0,
                at: std::time::SystemTime::now(),
            }],
            &TransientPolicy::from(&settings),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        let base = manager.generation();
        manager.poll(now);
        assert_ne!(manager.generation(), base);
        assert!(matches!(
            presenter.sync(
                &manager,
                sync_input(chrome_of(&settings), now, 960, 720, 1.0, Default::default())
            ),
            OsdSync::Render(_)
        ));
        // Past the Info duration the row expires: same two-phase order
        // must surface Clear, not Skip, then settle.
        let later = now + Duration::from_millis(2_500);
        let base = manager.generation();
        manager.poll(later);
        assert_ne!(manager.generation(), base);
        assert_ne!(manager.generation(), base);
        assert_ne!(manager.generation(), base);
        assert!(matches!(
            presenter.sync(
                &manager,
                sync_input(
                    chrome_of(&settings),
                    later,
                    960,
                    720,
                    1.0,
                    Default::default()
                )
            ),
            OsdSync::Clear
        ));
        assert!(matches!(
            presenter.sync(
                &manager,
                sync_input(
                    chrome_of(&settings),
                    later,
                    960,
                    720,
                    1.0,
                    Default::default()
                )
            ),
            OsdSync::Skip
        ));
    }

    #[test]
    fn status_reenable_passes_fresh_value() {
        use super::{OsdPresenter, OsdSync};
        use nerust_render_traits::renderer::SafeInsets;
        use std::time::Duration;
        let manager = OsdManager::new();
        let mut presenter = OsdPresenter::new();
        let mut settings = presenter_settings();
        settings.status_enabled = true;
        let now = std::time::Instant::now();
        assert!(matches!(
            presenter.sync(
                &manager,
                super::OsdSyncInput {
                    chrome: super::OsdChrome::from(&settings),
                    language: AppLanguage::English,
                    fps: 60.0,
                    paused: false,
                    now,
                    width: 960,
                    height: 720,
                    ui_scale: 1.0,
                    safe_insets: SafeInsets::default(),
                }
            ),
            OsdSync::Render(_)
        ));
        // Disabled: the live layer clears, then idles.
        settings.status_enabled = false;
        let off = now + Duration::from_millis(100);
        assert!(matches!(
            presenter.sync(
                &manager,
                super::OsdSyncInput {
                    chrome: super::OsdChrome::from(&settings),
                    language: AppLanguage::English,
                    fps: 61.0,
                    paused: false,
                    now: off,
                    width: 960,
                    height: 720,
                    ui_scale: 1.0,
                    safe_insets: SafeInsets::default(),
                }
            ),
            OsdSync::Clear
        ));
        // Re-enabled within the throttle window: fresh digits, not stale.
        settings.status_enabled = true;
        let plan = match presenter.sync(
            &manager,
            super::OsdSyncInput {
                chrome: super::OsdChrome::from(&settings),
                language: AppLanguage::English,
                fps: 61.0,
                paused: false,
                now: off,
                width: 960,
                height: 720,
                ui_scale: 1.0,
                safe_insets: SafeInsets::default(),
            },
        ) {
            OsdSync::Render(plan) => plan,
            other => panic!("expected render, got {other:?}"),
        };
        assert_eq!(
            plan.status.as_ref().map(|(l, v)| (l.as_str(), v.as_str())),
            Some(("", "61.0 fps"))
        );
    }

    #[test]
    fn take_modal_text_joins_bullets_and_drains() {
        let mut manager = OsdManager::new();
        assert_eq!(manager.take_modal_text(), None);
        for slot in [1, 2] {
            manager.push_all_with_policy(
                vec![UiNotification {
                    level: NotificationLevel::Error,
                    kind: NotificationKind::SlotFailed {
                        slot: Some(slot),
                        reason: SlotOpFailure::Storage,
                    },
                    count: 1,
                    seq: 0,
                    at: std::time::SystemTime::now(),
                }],
                &policy(),
                AppLanguage::English,
                Instant::now(),
                FrameChannel::Composited,
            );
        }
        let detail = manager.take_modal_text().expect("joined batch");
        assert!(detail.lines().count() == 2);
        assert!(detail.lines().all(|line| line.starts_with("• ")));
        assert_eq!(manager.take_modal_text(), None);
    }
}

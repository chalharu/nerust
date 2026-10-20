//! Foreground lifecycle: window-ready retry plus auto-pause bookkeeping.
//! Owns the retry state machine and the suspend/resume helpers.

use std::time::{Duration, Instant};

use crate::lifecycle_decision::{self, LifecyclePolicy, SuspendFileAction};

const FOREGROUND_RETRY_BASE_DELAY: Duration = Duration::from_millis(250);
const FOREGROUND_RETRY_MAX_DELAY: Duration = Duration::from_secs(2);
const FOREGROUND_RETRY_MAX_ATTEMPTS: u32 = 20;
/// Backoff shift cap: `1 << attempts` saturates at this shift so the delay
/// table stays explicit (`250ms, 500ms, 1s, 2s, 2s, ...`).
const BACKOFF_STEPS: u32 = 3;

/// Auto-pause + restore + foreground-retry state.
/// Attempts back off and give up after the cap.
pub(super) struct Lifecycle {
    auto_paused: bool,
    restore_pending: bool,
    resume_pending: bool,
    retry_attempts: u32,
    retry_at: Option<Instant>,
}

impl Lifecycle {
    pub(super) fn new(restore_pending: bool) -> Self {
        Self {
            auto_paused: false,
            restore_pending,
            resume_pending: false,
            retry_attempts: 0,
            retry_at: None,
        }
    }

    pub(super) fn restore_pending(&self) -> bool {
        self.restore_pending
    }

    pub(super) fn auto_paused(&self) -> bool {
        self.auto_paused
    }

    pub(super) fn clear_restore_pending(&mut self) {
        self.restore_pending = false;
    }

    pub(super) fn set_auto_paused(&mut self, paused: bool) {
        self.auto_paused = paused;
    }

    pub(super) fn resume_pending(&self) -> bool {
        self.resume_pending
    }

    /// 1-based index of the attempt about to run (attempts + 1).
    pub(super) fn attempt_index(&self) -> u32 {
        self.retry_attempts + 1
    }

    pub(super) fn begin_foreground_resume(&mut self) {
        log::info!(
            "begin_foreground_resume: restore_pending={} auto_paused={}",
            self.restore_pending,
            self.auto_paused,
        );
        self.resume_pending = true;
        self.retry_attempts = 0;
        self.retry_at = None;
    }

    pub(super) fn cancel_foreground_resume(&mut self) {
        self.resume_pending = false;
        self.retry_attempts = 0;
        self.retry_at = None;
    }

    pub(super) fn schedule_retry(&mut self, is_resumed: bool) -> bool {
        if !is_resumed || !self.resume_pending {
            return false;
        }
        if self.retry_attempts >= FOREGROUND_RETRY_MAX_ATTEMPTS {
            self.resume_pending = false;
            log::error!(
                "giving up after {FOREGROUND_RETRY_MAX_ATTEMPTS} Android window initialization attempts"
            );
            return false;
        }

        // Saturating shift: the delay can never wrap to zero and busy-spin.
        let shift = self.retry_attempts.min(BACKOFF_STEPS);
        let multiplier = 1_u32.checked_shl(shift).unwrap_or(1 << BACKOFF_STEPS);
        let delay = FOREGROUND_RETRY_BASE_DELAY
            .saturating_mul(multiplier)
            .min(FOREGROUND_RETRY_MAX_DELAY);
        self.retry_attempts += 1;
        self.retry_at = Some(Instant::now() + delay);
        log::info!(
            "schedule_foreground_retry: scheduled attempt {}/{} in {:?}",
            self.retry_attempts,
            FOREGROUND_RETRY_MAX_ATTEMPTS,
            delay
        );
        true
    }

    /// Deadline the event loop should wait for before the next attempt,
    /// if a retry is already scheduled.
    pub(super) fn retry_deadline(&self) -> Option<Instant> {
        self.retry_at
    }

    pub(super) fn clear_retry_deadline(&mut self) {
        self.retry_at = None;
    }

    pub(super) fn record_success(&mut self) {
        self.resume_pending = false;
        self.retry_attempts = 0;
        self.retry_at = None;
    }

    /// Restore policy for the current flags: the single decision point
    /// for the fetch lane and both resume gates.
    pub(super) fn policy(
        &self,
        restore_hidden_state: bool,
        error_paused: bool,
        has_document_reference: bool,
        has_library_id: bool,
    ) -> LifecyclePolicy {
        LifecyclePolicy {
            restore_hidden_state,
            lifecycle_auto_paused: self.auto_paused,
            error_paused,
            has_document_reference,
            has_library_id,
        }
    }

    /// Auto-pause only with a pausable session. The caller executes the pause;
    /// this type never touches command execution.
    pub(super) fn should_auto_pause(&self, can_pause: bool) -> bool {
        lifecycle_decision::should_lifecycle_pause(self.auto_paused, can_pause)
    }

    /// Pure suspend decision: records the hidden-save outcome and returns
    /// the marker action. The frontend executes the storage and session I/O.
    pub(super) fn save_decision(&mut self, saved_hidden: bool) -> SuspendFileAction {
        self.restore_pending = saved_hidden;
        lifecycle_decision::suspend_file_action(saved_hidden)
    }
}

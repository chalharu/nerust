//! Foreground lifecycle: window-ready retry plus auto-pause bookkeeping.
//!
//! Split out of the `AndroidFrontend` god object. Owns the resume-retry
//! state machine and the suspend/resume helpers; the frontend keeps wiring
//! (event loop, session, storage) and executes the decisions.

use std::time::{Duration, Instant};

use nerust_gui_shell::session::SessionHandle;

use super::storage::AndroidStorage;
use crate::lifecycle_decision::{self, ResumeWant};

const FOREGROUND_RETRY_BASE_DELAY: Duration = Duration::from_millis(250);
const FOREGROUND_RETRY_MAX_DELAY: Duration = Duration::from_secs(2);
const FOREGROUND_RETRY_MAX_ATTEMPTS: u32 = 20;

/// Auto-pause + restore + foreground-retry state. The retry loop exists
/// because the Android surface may not be ready on the first `resumed`
/// tick; attempts back off and give up loudly after the cap.
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
            self.auto_paused
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
                "giving up after {} Android window initialization attempts",
                FOREGROUND_RETRY_MAX_ATTEMPTS
            );
            return false;
        }

        let delay = FOREGROUND_RETRY_BASE_DELAY
            .saturating_mul(1_u32 << self.retry_attempts.min(3))
            .min(FOREGROUND_RETRY_MAX_DELAY);
        self.retry_attempts += 1;
        self.retry_at = Some(Instant::now() + delay);
        log::info!(
            "schedule_foreground_retry: scheduled attempt {} in {:?}",
            self.retry_attempts,
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

    /// Resume gate for a fresh ROM load: always resume, never across an
    /// error auto-pause (its dismiss owns that); a manual pause predates
    /// the suspend and must survive it.
    pub(super) fn resume_want_after_restore(
        &self,
        restore_hidden_state: bool,
        error_paused: bool,
    ) -> bool {
        let want =
            lifecycle_decision::should_resume_after_restore(restore_hidden_state, self.auto_paused);
        let want = want.then_some(ResumeWant::FreshLoad);
        lifecycle_decision::should_resume_on_foreground(lifecycle_decision::ForegroundResume {
            error_paused,
            want,
        })
    }

    /// Resume gate for the foreground retry: only our own lifecycle
    /// auto-pause, never across an error auto-pause.
    pub(super) fn resume_want_foreground_retry(&self, error_paused: bool) -> bool {
        let want = self.auto_paused.then_some(ResumeWant::LifecycleRetry);
        lifecycle_decision::should_resume_on_foreground(lifecycle_decision::ForegroundResume {
            error_paused,
            want,
        })
    }

    /// Auto-pause only with a pausable session; the predicate pins the
    /// contract (host-tested in `lifecycle_decision`). The caller executes
    /// the pause through `PauseControl` and records it via
    /// `set_auto_paused`, so this type never touches command execution.
    pub(super) fn should_auto_pause(&self, session: &SessionHandle) -> bool {
        lifecycle_decision::should_lifecycle_pause(
            self.auto_paused,
            session.tick_metrics().can_pause(),
        )
    }

    pub(super) fn save_for_suspend(
        &mut self,
        session: &mut SessionHandle,
        storage: &AndroidStorage,
    ) {
        log::info!(
            "save_lifecycle_state: paused={} lifecycle_auto_paused={} restore_pending={}",
            session.tick_metrics().paused,
            self.auto_paused,
            self.restore_pending
        );
        // Auto-pause is decided and executed by the caller
        // (`should_auto_pause` + `PauseControl`) before this runs, so the
        // pause flag here only reflects already-recorded state.
        session.clear_input();
        self.restore_pending = session.save_hidden_lifecycle_state();
        if !self.restore_pending {
            session.clear_hidden_lifecycle_state();
            storage.clear_restore_pending();
            log::info!("save_lifecycle_state: no hidden lifecycle state was produced");
        } else {
            storage.touch_restore_pending();
            log::info!("save_lifecycle_state: hidden lifecycle state saved");
        }
        session.flush_before_exit();
        log::info!("save_lifecycle_state: flushed session state");
    }
}

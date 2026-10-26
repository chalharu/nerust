//! Deferred auto-pause protocol: blur arms, arrival cancels-or-resumes,
//! commit lands the pause. Toolkit-free: tao feeds aux-window kinds,
//! gtk feeds main-window events only.

/// Focus-domain effect: what the host must do after an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusEffect {
    Pause,
    Resume,
}

/// The only inputs the arbiter accepts. Remote arrivals carry whether
/// that surface resumes (debugger/PPU style) or only cancels a pending
/// pause (settings/history style).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusEvent {
    Main(bool),
    Remote {
        focused: bool,
        resumes_on_arrival: bool,
    },
    Commit,
}

/// Main-window focus truth plus the deferred auto-pause protocol.
/// Pauses land only via commit; arrivals report a resume at most once.
#[derive(Debug)]
pub struct FocusArbiter {
    /// Main window holds focus.
    main_active: bool,
    /// Session paused by the protocol, never by the user directly.
    auto_paused: bool,
    /// Blur armed a pause; committed unless cancelled.
    pending: bool,
}

impl Default for FocusArbiter {
    fn default() -> Self {
        Self::new()
    }
}

impl FocusArbiter {
    pub fn new() -> Self {
        Self {
            main_active: true,
            auto_paused: false,
            pending: false,
        }
    }

    pub fn main_active(&self) -> bool {
        self.main_active
    }

    pub fn on(&mut self, event: FocusEvent) -> Option<FocusEffect> {
        match event {
            FocusEvent::Main(focused) => {
                if focused {
                    self.arrive().then_some(FocusEffect::Resume)
                } else {
                    self.leave_main();
                    None
                }
            }
            FocusEvent::Remote {
                focused,
                resumes_on_arrival,
            } => {
                if focused {
                    if resumes_on_arrival {
                        self.arrive_aux().then_some(FocusEffect::Resume)
                    } else {
                        // Cancel only: the auto-pause flag is preserved
                        // so a later main-window arrival still resumes.
                        self.cancel_pending();
                        None
                    }
                } else {
                    self.leave_remote();
                    None
                }
            }
            FocusEvent::Commit => {
                if self.take_armed() {
                    // Armed and uncancelled: the caller commits the pause.
                    Some(FocusEffect::Pause)
                } else {
                    None
                }
            }
        }
    }

    /// Record a protocol-driven pause (user pauses never set this).
    pub fn note_protocol_pause(&mut self) {
        self.auto_paused = true;
    }

    fn arrive(&mut self) -> bool {
        self.main_active = true;
        self.pending = false;
        std::mem::replace(&mut self.auto_paused, false)
    }

    fn leave_main(&mut self) {
        self.main_active = false;
        self.pending = true;
    }

    fn arrive_aux(&mut self) -> bool {
        self.arrive()
    }

    fn leave_remote(&mut self) {
        self.pending = true;
    }

    fn cancel_pending(&mut self) {
        self.pending = false;
    }

    fn take_armed(&mut self) -> bool {
        std::mem::take(&mut self.pending)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn main_blur_commits_pause_at_commit() {
        let mut arbiter = FocusArbiter::new();
        assert_eq!(arbiter.on(FocusEvent::Main(false)), None);
        assert_eq!(arbiter.on(FocusEvent::Commit), Some(FocusEffect::Pause));
        // Consumed: a second commit without re-arm is quiet.
        assert_eq!(arbiter.on(FocusEvent::Commit), None);
    }

    #[test]
    fn same_app_arrival_cancels_pause() {
        let mut arbiter = FocusArbiter::new();
        assert_eq!(arbiter.on(FocusEvent::Main(false)), None);
        // Same-burst remote arrival cancels the armed pause.
        assert_eq!(
            arbiter.on(FocusEvent::Remote {
                focused: true,
                resumes_on_arrival: true
            }),
            None
        );
        assert_eq!(arbiter.on(FocusEvent::Commit), None);
    }

    #[test]
    fn resume_reported_exactly_once() {
        let mut arbiter = FocusArbiter::new();
        assert_eq!(
            arbiter.on(FocusEvent::Remote {
                focused: false,
                resumes_on_arrival: true
            }),
            None
        );
        assert_eq!(arbiter.on(FocusEvent::Commit), Some(FocusEffect::Pause));
        arbiter.note_protocol_pause();
        assert_eq!(
            arbiter.on(FocusEvent::Main(true)),
            Some(FocusEffect::Resume)
        );
        assert_eq!(arbiter.on(FocusEvent::Main(true)), None);
    }

    #[test]
    fn non_resuming_arrival_preserves_resume() {
        let mut arbiter = FocusArbiter::new();
        assert_eq!(
            arbiter.on(FocusEvent::Remote {
                focused: false,
                resumes_on_arrival: false
            }),
            None
        );
        assert_eq!(arbiter.on(FocusEvent::Commit), Some(FocusEffect::Pause));
        arbiter.note_protocol_pause();
        // Non-resuming arrival cancels without resuming.
        assert_eq!(
            arbiter.on(FocusEvent::Remote {
                focused: true,
                resumes_on_arrival: false
            }),
            None
        );
        assert_eq!(
            arbiter.on(FocusEvent::Main(true)),
            Some(FocusEffect::Resume)
        );
    }
}

pub trait EmulationPort {
    fn loaded(&self) -> bool;
    fn paused(&self) -> bool;
    fn set_execution_held(&mut self, held: bool);
    fn pause(&mut self);
    fn resume(&mut self);
    fn flush(&mut self);
}

pub trait MediaPort {
    fn load(&mut self, path: &std::path::Path) -> Result<(), String>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionAction {
    Reset,
    Unload,
    SaveState,
    LoadState,
    NextSlot,
    PreviousSlot,
}

pub trait SessionActionPort {
    fn execute(&mut self, action: SessionAction) -> Result<bool, String>;
}

pub trait SettingsPort {
    type Settings: Clone;
    fn settings(&self) -> &Self::Settings;
    fn apply_settings(&mut self, settings: Self::Settings) -> Result<(), String>;
}

pub trait FramePort {
    type Frame;
    fn advance_frame(&mut self);
    fn frame(&self) -> Option<&Self::Frame>;
}

pub trait KeyboardPort {
    type Key;
    type Shortcut;
    fn key_event(
        &mut self,
        key: Self::Key,
        pressed: bool,
        controller_enabled: bool,
    ) -> Option<Self::Shortcut>;
    fn clear_input(&mut self);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PauseReason {
    User,
    Focus,
    Settings,
    Modal,
    Debugger,
}

impl PauseReason {
    fn bit(self) -> u8 {
        1 << self as u8
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ApplicationState {
    pub loaded: bool,
    pub paused: bool,
    pub user_paused: bool,
    pub closed: bool,
}

pub struct Application<P: EmulationPort> {
    port: P,
    holds: u8,
    owner: std::sync::Arc<()>,
    leases: std::collections::BTreeMap<u64, PauseReason>,
    next_lease: u64,
    closed: bool,
}

pub struct PauseLease {
    owner: std::sync::Arc<()>,
    sequence: u64,
}

impl<P: EmulationPort> Application<P> {
    pub fn new(mut port: P) -> Self {
        let holds = if port.loaded() && port.paused() {
            PauseReason::User.bit()
        } else {
            0
        };
        port.set_execution_held(holds != 0);
        Self {
            port,
            holds,
            owner: std::sync::Arc::new(()),
            leases: Default::default(),
            next_lease: 0,
            closed: false,
        }
    }

    pub fn state(&self) -> ApplicationState {
        ApplicationState {
            loaded: self.port.loaded(),
            paused: self.port.paused(),
            user_paused: self.holds(PauseReason::User),
            closed: self.closed,
        }
    }

    pub fn session(&self) -> &P {
        &self.port
    }

    pub fn holds(&self, reason: PauseReason) -> bool {
        self.holds & reason.bit() != 0 || self.leases.values().any(|held| *held == reason)
    }

    pub fn acquire_pause(&mut self, reason: PauseReason) -> Option<PauseLease> {
        if self.closed {
            return None;
        }
        let sequence = self.next_lease.checked_add(1)?;
        self.next_lease = sequence;
        self.leases.insert(sequence, reason);
        self.reconcile();
        Some(PauseLease {
            owner: self.owner.clone(),
            sequence,
        })
    }

    pub fn release_pause(&mut self, lease: PauseLease) -> bool {
        if self.closed
            || !std::sync::Arc::ptr_eq(&self.owner, &lease.owner)
            || self.leases.remove(&lease.sequence).is_none()
        {
            return false;
        }
        self.reconcile();
        true
    }

    pub fn set_pause(&mut self, reason: PauseReason, active: bool) {
        if self.closed || (self.holds & reason.bit() != 0) == active {
            return;
        }
        if active {
            self.holds |= reason.bit();
        } else {
            self.holds &= !reason.bit();
        }
        self.reconcile();
    }

    pub fn toggle_pause(&mut self) {
        self.set_pause(PauseReason::User, self.holds & PauseReason::User.bit() == 0);
    }

    pub fn close(&mut self) {
        if !self.closed {
            self.closed = true;
            self.port.flush();
        }
    }

    fn reconcile(&mut self) {
        let held = self.execution_held();
        self.port.set_execution_held(held);
        if self.port.loaded() && self.port.paused() != held {
            if held {
                self.port.pause();
            } else {
                self.port.resume();
            }
        }
    }

    fn execution_held(&self) -> bool {
        self.holds != 0 || !self.leases.is_empty()
    }
}

impl<P: EmulationPort + MediaPort> Application<P> {
    pub fn load_media(&mut self, path: &std::path::Path) -> Result<(), String> {
        if self.closed {
            return Err("application is closed".into());
        }
        self.port.set_execution_held(self.execution_held());
        let result = self.port.load(path);
        self.reconcile();
        result
    }
}

impl<P: EmulationPort + SessionActionPort> Application<P> {
    pub fn execute(&mut self, action: SessionAction) -> Result<bool, String> {
        if self.closed || !self.port.loaded() {
            return Ok(false);
        }
        let result = self.port.execute(action);
        self.reconcile();
        result
    }
}

impl<P: EmulationPort + SettingsPort> Application<P> {
    pub fn settings(&self) -> &P::Settings {
        self.port.settings()
    }

    pub fn apply_settings(&mut self, settings: P::Settings) -> Result<(), String> {
        if self.closed {
            return Err("application is closed".into());
        }
        self.port.set_execution_held(self.execution_held());
        let result = self.port.apply_settings(settings);
        self.reconcile();
        result
    }
}

impl<P: EmulationPort + FramePort> Application<P> {
    pub fn advance_frame(&mut self) -> Option<&P::Frame> {
        if self.closed || !self.port.loaded() {
            return None;
        }
        self.port.advance_frame();
        self.port.frame()
    }
}

impl<P: EmulationPort + KeyboardPort> Application<P> {
    pub fn key_event(&mut self, key: P::Key, pressed: bool) -> Option<P::Shortcut> {
        if self.closed {
            return None;
        }
        let controller_enabled =
            self.port.loaded() && !self.port.paused() && !self.execution_held();
        self.port.key_event(key, pressed, controller_enabled)
    }

    pub fn clear_input(&mut self) {
        if !self.closed {
            self.port.clear_input();
        }
    }
}

impl<P: EmulationPort> Drop for Application<P> {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, rc::Rc};

    struct Probe {
        paused: bool,
        held: bool,
        flushes: Rc<Cell<usize>>,
    }

    impl EmulationPort for Probe {
        fn loaded(&self) -> bool {
            true
        }
        fn paused(&self) -> bool {
            self.paused
        }
        fn set_execution_held(&mut self, held: bool) {
            self.held = held;
        }
        fn pause(&mut self) {
            self.paused = true;
        }
        fn resume(&mut self) {
            self.paused = false;
        }
        fn flush(&mut self) {
            self.flushes.set(self.flushes.get() + 1);
        }
    }

    impl KeyboardPort for Probe {
        type Key = ();
        type Shortcut = bool;
        fn key_event(&mut self, _: (), pressed: bool, controller_enabled: bool) -> Option<bool> {
            Some(pressed && controller_enabled)
        }
        fn clear_input(&mut self) {}
    }

    #[test]
    fn paused_keyboard_still_reaches_shortcut_port_without_controller_input() {
        let mut app = Application::new(Probe {
            paused: false,
            held: false,
            flushes: Rc::default(),
        });
        assert_eq!(app.key_event((), true), Some(true));
        let lease = app.acquire_pause(PauseReason::Settings).unwrap();
        assert_eq!(app.key_event((), true), Some(false));
        app.release_pause(lease);
        assert_eq!(app.key_event((), true), Some(true));
        app.close();
        assert_eq!(app.key_event((), true), None);
    }

    #[test]
    fn releasing_settings_and_focus_preserves_user_pause() {
        let mut app = Application::new(Probe {
            paused: false,
            held: false,
            flushes: Rc::default(),
        });
        app.toggle_pause();
        app.set_pause(PauseReason::Settings, true);
        app.set_pause(PauseReason::Focus, true);
        app.set_pause(PauseReason::Settings, false);
        app.set_pause(PauseReason::Focus, false);
        assert!(app.state().paused);
        assert!(app.holds(PauseReason::User));
        app.toggle_pause();
        assert!(!app.state().paused);
    }

    #[test]
    fn preexisting_pause_is_adopted_and_close_flushes_once() {
        let flushes = Rc::new(Cell::new(0));
        {
            let mut app = Application::new(Probe {
                paused: true,
                held: false,
                flushes: flushes.clone(),
            });
            app.set_pause(PauseReason::Settings, true);
            app.set_pause(PauseReason::Settings, false);
            assert!(app.state().paused);
            app.close();
            app.close();
            app.toggle_pause();
            assert!(app.state().closed);
        }
        assert_eq!(flushes.get(), 1);
    }

    #[test]
    fn old_window_release_does_not_resume_a_new_window_or_user_pause() {
        let mut app = Application::new(Probe {
            paused: false,
            held: false,
            flushes: Rc::default(),
        });
        let first = app.acquire_pause(PauseReason::Settings).unwrap();
        let second = app.acquire_pause(PauseReason::Settings).unwrap();
        app.set_pause(PauseReason::Settings, false);
        assert!(app.state().paused);
        assert!(app.release_pause(first));
        assert!(app.state().paused);
        app.toggle_pause();
        assert!(app.release_pause(second));
        assert!(app.state().paused);
        app.toggle_pause();
        assert!(!app.state().paused);
    }

    #[test]
    fn leases_cannot_release_another_application_and_closed_apps_reject_acquisition() {
        let mut first = Application::new(Probe {
            paused: false,
            held: false,
            flushes: Rc::default(),
        });
        let mut second = Application::new(Probe {
            paused: false,
            held: false,
            flushes: Rc::default(),
        });
        let lease = first.acquire_pause(PauseReason::Modal).unwrap();
        let own = second.acquire_pause(PauseReason::Modal).unwrap();
        assert!(!second.release_pause(lease));
        assert!(first.state().paused && second.state().paused);
        assert!(second.release_pause(own));
        second.close();
        assert!(second.acquire_pause(PauseReason::Settings).is_none());
    }

    struct TransactionProbe {
        base: Probe,
        loaded: bool,
        settings: u8,
        fail: bool,
        swaps: usize,
    }

    fn transaction() -> TransactionProbe {
        TransactionProbe {
            base: Probe {
                paused: false,
                held: false,
                flushes: Rc::default(),
            },
            loaded: false,
            settings: 0,
            fail: false,
            swaps: 0,
        }
    }

    impl EmulationPort for TransactionProbe {
        fn loaded(&self) -> bool {
            self.loaded
        }
        fn paused(&self) -> bool {
            self.base.paused
        }
        fn set_execution_held(&mut self, held: bool) {
            self.base.set_execution_held(held);
        }
        fn pause(&mut self) {
            self.base.pause();
        }
        fn resume(&mut self) {
            self.base.resume();
        }
        fn flush(&mut self) {
            self.base.flush();
        }
    }

    impl MediaPort for TransactionProbe {
        fn load(&mut self, _: &std::path::Path) -> Result<(), String> {
            self.loaded = true;
            self.base.paused = false;
            if self.fail {
                Err("media failed".into())
            } else {
                Ok(())
            }
        }
    }

    impl SettingsPort for TransactionProbe {
        type Settings = u8;
        fn settings(&self) -> &u8 {
            &self.settings
        }
        fn apply_settings(&mut self, settings: u8) -> Result<(), String> {
            self.settings = settings;
            self.base.paused = false;
            if self.fail {
                Err("settings failed".into())
            } else {
                Ok(())
            }
        }
    }

    impl SessionActionPort for TransactionProbe {
        fn execute(&mut self, action: SessionAction) -> Result<bool, String> {
            self.base.paused = false;
            if action == SessionAction::Unload {
                self.loaded = false;
            }
            Ok(true)
        }
    }

    impl FramePort for TransactionProbe {
        type Frame = usize;
        fn advance_frame(&mut self) {
            self.swaps += 1;
        }
        fn frame(&self) -> Option<&usize> {
            Some(&self.swaps)
        }
    }

    #[test]
    fn load_and_settings_reconcile_pause_even_after_a_port_failure() {
        for fail in [false, true] {
            let mut port = transaction();
            port.fail = fail;
            let mut app = Application::new(port);
            let lease = app.acquire_pause(PauseReason::Settings).unwrap();
            let loaded = app.load_media(std::path::Path::new("rom"));
            assert_eq!(loaded.is_err(), fail);
            assert!(app.state().paused && app.session().base.held);
            let applied = app.apply_settings(42);
            assert_eq!(applied.is_err(), fail);
            assert!(app.state().paused && app.session().base.held);
            assert!(app.release_pause(lease));
            assert!(!app.state().paused && !app.session().base.held);
        }
    }

    #[test]
    fn unloaded_and_closed_applications_reject_session_and_frame_work() {
        let mut app = Application::new(transaction());
        assert_eq!(app.advance_frame(), None);
        assert!(!app.execute(SessionAction::Reset).unwrap());
        app.load_media(std::path::Path::new("rom")).unwrap();
        assert_eq!(app.advance_frame(), Some(&1));
        app.toggle_pause();
        assert!(app.execute(SessionAction::Reset).unwrap());
        assert!(app.state().paused);
        app.close();
        assert_eq!(app.advance_frame(), None);
        assert!(!app.execute(SessionAction::Unload).unwrap());
        assert!(app.load_media(std::path::Path::new("other")).is_err());
        assert!(app.apply_settings(12).is_err());
        assert_eq!(*app.settings(), 0);
        assert_eq!(app.session().swaps, 1);
    }

    #[test]
    fn unloaded_media_retains_pause_ownership_for_the_next_load() {
        let mut app = Application::new(transaction());
        app.toggle_pause();
        app.load_media(std::path::Path::new("first")).unwrap();
        assert!(app.execute(SessionAction::Unload).unwrap());
        assert!(!app.state().loaded);
        assert!(app.holds(PauseReason::User));
        assert_eq!(app.advance_frame(), None);
        app.load_media(std::path::Path::new("second")).unwrap();
        assert!(app.state().paused);
    }
}

//! Toolkit-independent session event delivery and composed pause ownership.
//! The driver borrows existing host fields; it adds no native lifetime or widget.

use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_gui_shell::session::{
    KeyboardShortcut, SessionHandle, access::SettingsResult, commands::SessionCommand,
    commands::SessionCommandOutcome,
};
use nerust_settings_core::model::input::ShortcutAction;
use wakui::pause::{PauseOwner, PausePolicy};
use wakui::session::{
    CommandOutcome, SessionEffect, SessionError, SessionEvent, SessionOutcome, ShortcutGesture,
};

use crate::commands::{CommandTable, MenuCommand};

fn convert_error(error: nerust_gui_shell::session::SessionError) -> SessionError {
    SessionError(error.to_string())
}

fn gesture(shortcut: KeyboardShortcut) -> ShortcutGesture {
    match shortcut {
        KeyboardShortcut::OpenDebugger => ShortcutGesture::OpenDebugger,
        KeyboardShortcut::ToggleFullscreen => ShortcutGesture::ToggleFullscreen,
        KeyboardShortcut::Session(action) => match action {
            ShortcutAction::TogglePause => ShortcutGesture::TogglePause,
            ShortcutAction::SaveActiveSlot => ShortcutGesture::SaveActiveSlot,
            ShortcutAction::LoadActiveSlot => ShortcutGesture::LoadActiveSlot,
            ShortcutAction::SelectNextSlot => ShortcutGesture::SelectNextSlot,
            ShortcutAction::SelectPreviousSlot => ShortcutGesture::SelectPreviousSlot,
            ShortcutAction::Reset => ShortcutGesture::Reset,
            ShortcutAction::OpenDebugger => ShortcutGesture::OpenDebugger,
            ShortcutAction::ToggleFullscreen => ShortcutGesture::ToggleFullscreen,
        },
    }
}

pub struct SessionDriver<'a> {
    session: &'a mut SessionHandle,
    policy: &'a mut PausePolicy,
    table: &'a CommandTable,
}

impl<'a> SessionDriver<'a> {
    pub fn new(
        session: &'a mut SessionHandle,
        policy: &'a mut PausePolicy,
        table: &'a CommandTable,
    ) -> Self {
        Self {
            session,
            policy,
            table,
        }
    }

    /// Typed command entry: pause ownership plus shell execution. Opaque
    /// activations resolve here first; direct callers (debugger drain,
    /// shell ports) already hold typed commands.
    pub fn dispatch_command(
        &mut self,
        command: SessionCommand,
    ) -> Result<SessionOutcome, SessionError> {
        let mut outcome = SessionOutcome::default();
        let executed = match command {
            SessionCommand::Pause | SessionCommand::Resume | SessionCommand::TogglePause => {
                let active = match command {
                    SessionCommand::Pause => true,
                    SessionCommand::Resume => false,
                    _ => !self.session.tick_metrics().paused,
                };
                set_pause_owner(self.session, self.policy, PauseOwner::User, active)
            }
            command => self.session.run_command(command).map_err(convert_error)?,
        };
        outcome.command = Some(CommandOutcome {
            executed: executed.executed,
            needs_redraw: executed.needs_redraw,
        });
        Ok(outcome)
    }

    pub fn dispatch(&mut self, event: SessionEvent) -> Result<SessionOutcome, SessionError> {
        let mut outcome = SessionOutcome::default();
        match event {
            SessionEvent::Command(activation) => {
                // Stale ids (previous model) resolve to nothing and are ignored.
                let Some(MenuCommand::Session(command)) = self.table.resolve(activation) else {
                    return Ok(outcome);
                };
                return self.dispatch_command(command);
            }
            SessionEvent::Keyboard { key, pressed } => {
                if !self.policy.owns(PauseOwner::Settings)
                    && let Some(shortcut) = self.session.handle_keyboard_key(key, pressed)
                {
                    return self.dispatch(SessionEvent::Shortcut(gesture(shortcut)));
                }
            }
            SessionEvent::Shortcut(gesture) => match gesture {
                ShortcutGesture::OpenDebugger => {
                    outcome.effect = Some(SessionEffect::NativeOpenDebugger);
                }
                ShortcutGesture::ToggleFullscreen => {
                    outcome.effect = Some(SessionEffect::ToggleFullscreen);
                }
                ShortcutGesture::TogglePause => {
                    return self.dispatch_command(SessionCommand::TogglePause);
                }
                ShortcutGesture::SaveActiveSlot => {
                    return self.dispatch_command(SessionCommand::SaveActiveSlotOrNew);
                }
                ShortcutGesture::LoadActiveSlot => {
                    return self.dispatch_command(SessionCommand::LoadActiveSlot);
                }
                ShortcutGesture::SelectNextSlot => {
                    return self.dispatch_command(SessionCommand::SelectNextSlot);
                }
                ShortcutGesture::SelectPreviousSlot => {
                    return self.dispatch_command(SessionCommand::SelectPreviousSlot);
                }
                ShortcutGesture::Reset => {
                    return self.dispatch_command(SessionCommand::Reset);
                }
            },
            SessionEvent::ClearInput => self.session.clear_input(),
            SessionEvent::SettingsOpened => {
                if !self.policy.owns(PauseOwner::Settings) {
                    self.session.clear_input();
                    set_pause_owner(self.session, self.policy, PauseOwner::Settings, true);
                    outcome.settings_changed = true;
                }
            }
            SessionEvent::SettingsClosed | SessionEvent::SettingsOpenFailed => {
                if self.policy.owns(PauseOwner::Settings) {
                    set_pause_owner(self.session, self.policy, PauseOwner::Settings, false);
                    outcome.settings_changed = true;
                }
            }
        }
        Ok(outcome)
    }
}

pub fn dispatch(
    session: &mut SessionHandle,
    policy: &mut PausePolicy,
    table: &CommandTable,
    event: SessionEvent,
) -> Result<SessionOutcome, SessionError> {
    SessionDriver::new(session, policy, table).dispatch(event)
}

/// Typed command entry for direct callers holding domain commands
/// (debugger drain, shell ports). Opaque activations go through [`dispatch`].
pub fn dispatch_command(
    session: &mut SessionHandle,
    policy: &mut PausePolicy,
    command: SessionCommand,
) -> Result<SessionOutcome, SessionError> {
    SessionDriver::new(session, policy, &CommandTable::default()).dispatch_command(command)
}

/// Ownership changes are retained even without a core. The execution hold gates
/// load/apply activation before a worker can run; commands synchronize a live one.
pub fn set_pause_owner(
    session: &mut SessionHandle,
    policy: &mut PausePolicy,
    owner: PauseOwner,
    active: bool,
) -> SessionCommandOutcome {
    let tick = session.tick_metrics();
    let paused = policy.set(owner, active, tick.paused);
    session.set_execution_held(paused);
    if tick.loaded && tick.paused != paused {
        session
            .run_command(if paused {
                SessionCommand::Pause
            } else {
                SessionCommand::Resume
            })
            .unwrap_or_default()
    } else {
        SessionCommandOutcome::default()
    }
}

/// Reassert holds after media replacement or settings apply (including rollback).
/// With no owners, do not resume an independently paused/debugger-stopped core.
pub fn reconcile_pause(session: &mut SessionHandle, policy: &PausePolicy) {
    let tick = session.tick_metrics();
    session.set_execution_held(policy.paused());
    if policy.paused() && tick.loaded && !tick.paused {
        let _ = session.run_command(SessionCommand::Pause);
    }
}

pub fn apply_settings(
    session: &mut SessionHandle,
    policy: &PausePolicy,
    settings: SettingsSnapshot,
) -> Result<SettingsResult, nerust_gui_shell::session::SessionError> {
    reconcile_pause(session, policy);
    let result = session.apply_settings(settings);
    reconcile_pause(session, policy);
    let plan = result?;
    Ok(SettingsResult {
        renderer_needs_rebuild: plan.renderer_rebuild_required || plan.window_settings_changed,
        fullscreen_default_changed: plan.fullscreen_default_changed,
        scaling_changed: plan.scaling_changed,
    })
}

/// Both native hosts expose these narrow shell ports over the same two fields.
/// Keep the wiring here rather than copying four impl blocks into each toolkit.
#[macro_export]
macro_rules! impl_session_controls {
    ($host:ty) => {
        impl nerust_gui_shell::session::access::PauseControl for $host {
            fn pause(&mut self) {
                let _ = self.dispatch_session_command(
                    nerust_gui_shell::session::commands::SessionCommand::Pause,
                );
            }
            fn resume(&mut self) {
                let _ = self.dispatch_session_command(
                    nerust_gui_shell::session::commands::SessionCommand::Resume,
                );
            }
            fn toggle_pause(&mut self) {
                let _ = self.dispatch_session_command(
                    nerust_gui_shell::session::commands::SessionCommand::TogglePause,
                );
            }
        }
        impl nerust_gui_shell::session::access::SlotIo for $host {
            fn save_active_slot(&mut self) -> bool {
                self.dispatch_session_command(
                    nerust_gui_shell::session::commands::SessionCommand::SaveActiveSlotOrNew,
                )
                .ok()
                .and_then(|out| out.command)
                .is_some_and(|out| out.executed)
            }
            fn load_active_slot(&mut self) -> bool {
                self.dispatch_session_command(
                    nerust_gui_shell::session::commands::SessionCommand::LoadActiveSlot,
                )
                .ok()
                .and_then(|out| out.command)
                .is_some_and(|out| out.executed)
            }
            fn load_slot(&mut self, id: u64) -> bool {
                self.dispatch_session_command(
                    nerust_gui_shell::session::commands::SessionCommand::LoadSlot(id),
                )
                .ok()
                .and_then(|out| out.command)
                .is_some_and(|out| out.executed)
            }
            fn save_slot(&mut self, id: u64) {
                let _ = self.dispatch_session_command(
                    nerust_gui_shell::session::commands::SessionCommand::SaveSlot(id),
                );
            }
            fn delete_slot(&mut self, id: u64) {
                let _ = self.dispatch_session_command(
                    nerust_gui_shell::session::commands::SessionCommand::DeleteSlot(id),
                );
            }
        }
        impl nerust_gui_shell::session::access::SlotNav for $host {
            fn select_next_slot(&mut self) {
                let _ = self.dispatch_session_command(
                    nerust_gui_shell::session::commands::SessionCommand::SelectNextSlot,
                );
            }
            fn select_previous_slot(&mut self) {
                let _ = self.dispatch_session_command(
                    nerust_gui_shell::session::commands::SessionCommand::SelectPreviousSlot,
                );
            }
            fn select_slot(&mut self, id: u64) {
                let _ = self.dispatch_session_command(
                    nerust_gui_shell::session::commands::SessionCommand::SelectActiveSlot(id),
                );
            }
            fn create_slot(&mut self) {
                let _ = self.dispatch_session_command(
                    nerust_gui_shell::session::commands::SessionCommand::CreateSlot,
                );
            }
        }
        impl nerust_gui_shell::session::access::ResetControl for $host {
            fn reset(&mut self) {
                let _ = self.dispatch_session_command(
                    nerust_gui_shell::session::commands::SessionCommand::Reset,
                );
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use nerust_gui_runtime::settings::{
        HostBackendCapabilities, HostWindowCapabilities, SettingsPaths,
    };
    use nerust_gui_shell::{load::RomLoader, registry::SystemRegistry};
    use std::{
        path::PathBuf,
        sync::{
            Arc,
            atomic::{AtomicU64, Ordering},
        },
    };
    use wakui::Key;

    struct TestHost {
        session: SessionHandle,
        policy: PausePolicy,
        loader: Box<dyn RomLoader>,
        root: PathBuf,
    }

    impl TestHost {
        fn new(nes: bool) -> Self {
            static SEQUENCE: AtomicU64 = AtomicU64::new(0);
            let root = std::env::temp_dir().join(format!(
                "nerust-host-session-{}-{}",
                std::process::id(),
                SEQUENCE.fetch_add(1, Ordering::Relaxed),
            ));
            std::fs::create_dir(&root).unwrap();
            let registry = Arc::new(SystemRegistry::new(if nes {
                vec![nerust_nes_factory::NesFactory::boxed().into()]
            } else {
                vec![]
            }));
            let loader = registry.create_loader(Default::default()).unwrap();
            let session = SessionHandle::new_with_settings_paths(
                HostBackendCapabilities {
                    window: HostWindowCapabilities {
                        remembers_window_size: false,
                        supports_fullscreen_default: true,
                        supports_scaling: true,
                    },
                    presentation: None,
                },
                registry,
                Arc::new(nerust_core_traits::audio::AudioBackendRegistry::new()),
                SettingsPaths {
                    settings_file: root.join("settings.toml"),
                    central_storage_root: root.join("storage"),
                },
            )
            .unwrap();
            Self {
                session,
                policy: PausePolicy::default(),
                loader,
                root,
            }
        }

        fn dispatch_session(
            &mut self,
            event: SessionEvent,
        ) -> Result<SessionOutcome, SessionError> {
            dispatch(
                &mut self.session,
                &mut self.policy,
                &CommandTable::default(),
                event,
            )
        }

        fn dispatch_session_command(
            &mut self,
            command: SessionCommand,
        ) -> Result<SessionOutcome, SessionError> {
            dispatch_command(&mut self.session, &mut self.policy, command)
        }

        fn event(&mut self, event: SessionEvent) -> SessionOutcome {
            self.dispatch_session(event).unwrap()
        }

        fn command(&mut self, command: SessionCommand) -> SessionOutcome {
            self.dispatch_session_command(command).unwrap()
        }

        fn owner(&mut self, owner: PauseOwner, active: bool) {
            set_pause_owner(&mut self.session, &mut self.policy, owner, active);
        }

        fn load_nes(&mut self) {
            let mut rom = vec![0u8; 16 + 16384 + 8192];
            rom[..6].copy_from_slice(&[b'N', b'E', b'S', 0x1a, 1, 1]);
            rom[16..19].copy_from_slice(&[0x4c, 0x00, 0x80]);
            for vector in [0x3ffa, 0x3ffc, 0x3ffe] {
                rom[16 + vector..18 + vector].copy_from_slice(&[0x00, 0x80]);
            }
            let path = self.root.join("loop.nes");
            std::fs::write(&path, rom).unwrap();
            self.loader.load_rom(&path, &mut self.session).unwrap();
            reconcile_pause(&mut self.session, &self.policy);
            assert!(self.session.tick_metrics().loaded);
        }

        fn bind(&mut self, action: ShortcutAction, key: Key) {
            let mut settings = self.session.settings_snapshot().clone();
            settings.shared.input.shortcuts.keyboard =
                vec![nerust_settings_core::model::input::ShortcutBinding {
                    action,
                    key: Some(key),
                }];
            apply_settings(&mut self.session, &self.policy, settings).unwrap();
        }

        fn key(&mut self, key: Key, pressed: bool) -> SessionOutcome {
            self.event(SessionEvent::Keyboard { key, pressed })
        }
    }

    impl Drop for TestHost {
        fn drop(&mut self) {
            self.session.flush_before_exit();
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    crate::impl_session_controls!(TestHost);

    #[test]
    fn keyboard_effects_repeat_release_and_clear_are_shared() {
        let mut host = TestHost::new(false);
        for (action, effect) in [
            (
                ShortcutAction::OpenDebugger,
                SessionEffect::NativeOpenDebugger,
            ),
            (
                ShortcutAction::ToggleFullscreen,
                SessionEffect::ToggleFullscreen,
            ),
        ] {
            host.bind(action, Key::F2);
            assert_eq!(host.key(Key::F2, true).effect, Some(effect));
            assert!(
                host.key(Key::F2, true).effect.is_none(),
                "repeat is suppressed"
            );
            assert!(host.key(Key::F2, false).effect.is_none());
            assert_eq!(host.key(Key::F2, true).effect, Some(effect));
            host.event(SessionEvent::ClearInput);
            assert_eq!(host.key(Key::F2, true).effect, Some(effect));
            host.event(SessionEvent::ClearInput);
        }
    }

    #[test]
    fn settings_events_are_idempotent_and_clear_keys_without_widgets() {
        let mut host = TestHost::new(false);
        host.bind(ShortcutAction::OpenDebugger, Key::F2);
        assert!(host.key(Key::F2, true).effect.is_some());
        assert!(!host.event(SessionEvent::SettingsClosed).settings_changed);
        assert!(host.event(SessionEvent::SettingsOpened).settings_changed);
        assert!(!host.event(SessionEvent::SettingsOpened).settings_changed);
        assert!(host.key(Key::F2, true).effect.is_none());
        assert!(
            host.event(SessionEvent::SettingsOpenFailed)
                .settings_changed
        );
        assert!(!host.event(SessionEvent::SettingsClosed).settings_changed);
        assert!(
            !host
                .event(SessionEvent::SettingsOpenFailed)
                .settings_changed
        );
        assert!(!host.policy.paused());
        assert!(
            host.key(Key::F2, true).effect.is_some(),
            "open cleared the previous press"
        );
        assert!(
            host.event(SessionEvent::SettingsOpened).settings_changed,
            "failure permits retry"
        );
        assert!(host.event(SessionEvent::SettingsClosed).settings_changed);
    }

    #[test]
    fn shared_narrow_ports_remain_total_without_core() {
        use nerust_gui_shell::session::access::{PauseControl, ResetControl, SlotIo, SlotNav};
        let mut host = TestHost::new(false);
        host.pause();
        assert!(host.policy.owns(PauseOwner::User));
        host.resume();
        host.toggle_pause();
        host.resume();
        assert!(!host.policy.paused());
        assert!(!host.save_active_slot());
        assert!(!host.load_active_slot());
        assert!(!host.load_slot(7));
        host.save_slot(7);
        host.delete_slot(7);
        host.select_slot(7);
        assert_eq!(host.session.active_slot_id(), Some(7));
        host.select_next_slot();
        host.select_previous_slot();
        host.create_slot();
        host.reset();
        assert!(!host.session.tick_metrics().loaded);
    }

    #[test]
    fn keyboard_and_menu_commands_use_the_same_pause_owner() {
        let mut host = TestHost::new(true);
        host.load_nes();
        host.bind(ShortcutAction::TogglePause, Key::F2);
        assert!(host.key(Key::F2, true).command.unwrap().executed);
        assert!(host.policy.owns(PauseOwner::User));
        assert!(host.session.tick_metrics().paused);
        assert!(host.key(Key::F2, true).command.is_none());
        host.command(SessionCommand::Resume);
        assert!(!host.policy.paused());
        assert!(!host.session.tick_metrics().paused);
        host.key(Key::F2, false);
        host.key(Key::F2, true);
        assert!(host.policy.owns(PauseOwner::User));
    }

    #[test]
    fn settings_modal_focus_and_user_compose_across_real_nes_replacement_and_apply() {
        let mut host = TestHost::new(true);
        // An owner acquired with no core must gate the first load too.
        host.event(SessionEvent::SettingsOpened);
        host.load_nes();
        assert!(host.session.tick_metrics().paused);
        assert_eq!(host.session.tick_metrics().frame_counter, 0);
        host.owner(PauseOwner::Modal, true);
        host.owner(PauseOwner::Focus, true);
        host.load_nes();
        assert!(host.session.tick_metrics().paused);
        assert_eq!(host.session.tick_metrics().frame_counter, 0);
        let snapshot = host.session.settings_snapshot().clone();
        apply_settings(&mut host.session, &host.policy, snapshot).unwrap();
        let mut invalid = host.session.settings_snapshot().clone();
        invalid.local.audio.master_volume_percent = 101;
        assert!(apply_settings(&mut host.session, &host.policy, invalid).is_err());
        assert!(host.session.tick_metrics().paused);
        host.event(SessionEvent::SettingsClosed);
        host.event(SessionEvent::SettingsClosed);
        host.owner(PauseOwner::Focus, false);
        assert!(host.session.tick_metrics().paused, "modal still holds");
        host.command(SessionCommand::Pause);
        host.owner(PauseOwner::Modal, false);
        assert!(
            host.session.tick_metrics().paused,
            "explicit pause survives modal release"
        );
        host.command(SessionCommand::Resume);
        assert!(!host.session.tick_metrics().paused);
    }

    #[test]
    fn failed_open_rolls_back_only_settings_and_can_retry_with_real_nes() {
        let mut host = TestHost::new(true);
        host.load_nes();
        host.owner(PauseOwner::Modal, true);
        host.event(SessionEvent::SettingsOpened);
        host.event(SessionEvent::SettingsOpened);
        assert!(
            !host.policy.owns(PauseOwner::User),
            "duplicate open never adopts its own pause"
        );
        host.event(SessionEvent::SettingsOpenFailed);
        assert!(host.policy.owns(PauseOwner::Modal));
        assert!(!host.policy.owns(PauseOwner::Settings));
        assert!(host.session.tick_metrics().paused);
        host.owner(PauseOwner::Modal, false);
        assert!(!host.session.tick_metrics().paused);
        assert!(host.event(SessionEvent::SettingsOpened).settings_changed);
        assert!(host.session.tick_metrics().paused);
        host.event(SessionEvent::SettingsOpenFailed);
        assert!(!host.session.tick_metrics().paused);
    }

    #[test]
    fn observed_explicit_pause_is_adopted_but_duplicate_close_never_resumes_it() {
        let mut host = TestHost::new(true);
        host.load_nes();
        host.session.run_command(SessionCommand::Pause).unwrap();
        host.event(SessionEvent::SettingsOpened);
        assert!(host.policy.owns(PauseOwner::User));
        host.event(SessionEvent::SettingsClosed);
        assert!(host.session.tick_metrics().paused);
        host.command(SessionCommand::Resume);
        host.session.run_command(SessionCommand::Pause).unwrap();
        assert!(!host.event(SessionEvent::SettingsClosed).settings_changed);
        assert!(
            !host
                .event(SessionEvent::SettingsOpenFailed)
                .settings_changed
        );
        reconcile_pause(&mut host.session, &host.policy);
        assert!(
            host.session.tick_metrics().paused,
            "no owners is not a resume request"
        );
    }

    #[test]
    fn resume_under_settings_hold_clears_only_user_ownership() {
        let mut host = TestHost::new(true);
        host.load_nes();
        host.command(SessionCommand::Pause);
        host.event(SessionEvent::SettingsOpened);
        host.command(SessionCommand::Resume);
        assert!(!host.policy.owns(PauseOwner::User));
        assert!(host.session.tick_metrics().paused);
        host.event(SessionEvent::SettingsClosed);
        assert!(!host.session.tick_metrics().paused);
    }
}

//! Toolkit-independent session event delivery and composed pause ownership.
//! The driver borrows existing host fields; it adds no native lifetime or widget.
//!
//! Domain meanings live here, not in `wakui`: shortcuts (debugger, slots,
//! fullscreen), the settings open/close protocol, and the pause-owner table.
//! The framework only transports opaque [`wakui::input::session::SessionEvent`]
//! inputs; this module resolves them to typed shell commands.

use crate::pause::{PauseOwner, PausePolicy};
use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_gui_shell::session::{
    KeyboardShortcut, SessionHandle, access::SettingsResult, commands::SessionCommand,
    commands::SessionCommandOutcome,
};
use nerust_settings_core::model::input::ShortcutAction;
use wakui::input::session::{CommandOutcome, SessionError, SessionEvent, SessionOutcome};

use crate::commands::{CommandTable, MenuCommand};

/// View-side suspension owners. The framework holds an opaque bitset;
/// this table gives each hold its meaning. Order is load-bearing only
/// for readability; bits never alias.
pub mod pause_owners {
    use crate::pause::PauseOwner;

    pub const USER: PauseOwner = PauseOwner::new(0);
    pub const FOCUS: PauseOwner = PauseOwner::new(1);
    pub const MODAL: PauseOwner = PauseOwner::new(2);
    pub const SETTINGS: PauseOwner = PauseOwner::new(3);
}

/// View-side shortcut: what a shell keyboard binding means. Produced here
/// from [`KeyboardShortcut`], consumed here; flavors only forward
/// [`SessionEvent::Keyboard`] and never name these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewShortcut {
    OpenDebugger,
    ToggleFullscreen,
    TogglePause,
    SaveActiveSlot,
    LoadActiveSlot,
    SelectNextSlot,
    SelectPreviousSlot,
    Reset,
}

/// View-side session effect. Flavors never see these; [`crate::event`]
/// maps them to native [`wakui::input::event::CommandEffect`]s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverEffect {
    OpenDebugger,
    ToggleFullscreen,
}

/// Driver outcome: the framework [`SessionOutcome`] plus view-side routing.
/// The [`FrontendApp::dispatch`](wakui::app::boundary::FrontendApp::dispatch) boundary
/// exposes only the command half; effects and settings transitions stay here.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DriverOutcome {
    pub command: Option<CommandOutcome>,
    pub effect: Option<DriverEffect>,
    /// Whether this event acquired/released the Settings owner. Native chrome
    /// can skip duplicate close notifications using this flag.
    pub settings_changed: bool,
}

impl From<DriverOutcome> for SessionOutcome {
    fn from(outcome: DriverOutcome) -> Self {
        Self {
            command: outcome.command,
        }
    }
}

fn convert_error(error: nerust_gui_shell::session::SessionError) -> SessionError {
    SessionError(error.to_string())
}

fn gesture(shortcut: KeyboardShortcut) -> ViewShortcut {
    match shortcut {
        KeyboardShortcut::OpenDebugger => ViewShortcut::OpenDebugger,
        KeyboardShortcut::ToggleFullscreen => ViewShortcut::ToggleFullscreen,
        KeyboardShortcut::Session(action) => match action {
            ShortcutAction::TogglePause => ViewShortcut::TogglePause,
            ShortcutAction::SaveActiveSlot => ViewShortcut::SaveActiveSlot,
            ShortcutAction::LoadActiveSlot => ViewShortcut::LoadActiveSlot,
            ShortcutAction::SelectNextSlot => ViewShortcut::SelectNextSlot,
            ShortcutAction::SelectPreviousSlot => ViewShortcut::SelectPreviousSlot,
            ShortcutAction::Reset => ViewShortcut::Reset,
            ShortcutAction::OpenDebugger => ViewShortcut::OpenDebugger,
            ShortcutAction::ToggleFullscreen => ViewShortcut::ToggleFullscreen,
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
    ) -> Result<DriverOutcome, SessionError> {
        let mut outcome = DriverOutcome::default();
        let executed = match command {
            SessionCommand::Pause | SessionCommand::Resume | SessionCommand::TogglePause => {
                let active = match command {
                    SessionCommand::Pause => true,
                    SessionCommand::Resume => false,
                    _ => !self.session.tick_metrics().paused,
                };
                set_pause_owner(self.session, self.policy, pause_owners::USER, active)
            }
            command => self.session.run_command(command).map_err(convert_error)?,
        };
        outcome.command = Some(CommandOutcome {
            executed: executed.executed,
            needs_redraw: executed.needs_redraw,
        });
        Ok(outcome)
    }

    pub fn dispatch(&mut self, event: SessionEvent) -> Result<DriverOutcome, SessionError> {
        let outcome = DriverOutcome::default();
        match event {
            SessionEvent::Command(activation) => {
                // Stale ids (previous model) resolve to nothing and are ignored.
                let Some(MenuCommand::Session(command)) = self.table.resolve(activation) else {
                    return Ok(outcome);
                };
                return self.dispatch_command(command);
            }
            SessionEvent::Keyboard { key, pressed } => {
                if !self.policy.owns(pause_owners::SETTINGS)
                    && let Some(shortcut) = self.session.handle_keyboard_key(key, pressed)
                {
                    return self.shortcut(gesture(shortcut));
                }
            }
            SessionEvent::ClearInput => self.session.clear_input(),
        }
        Ok(outcome)
    }

    fn shortcut(&mut self, shortcut: ViewShortcut) -> Result<DriverOutcome, SessionError> {
        let mut outcome = DriverOutcome::default();
        match shortcut {
            ViewShortcut::OpenDebugger => {
                outcome.effect = Some(DriverEffect::OpenDebugger);
            }
            ViewShortcut::ToggleFullscreen => {
                outcome.effect = Some(DriverEffect::ToggleFullscreen);
            }
            ViewShortcut::TogglePause => {
                return self.dispatch_command(SessionCommand::TogglePause);
            }
            ViewShortcut::SaveActiveSlot => {
                return self.dispatch_command(SessionCommand::SaveActiveSlotOrNew);
            }
            ViewShortcut::LoadActiveSlot => {
                return self.dispatch_command(SessionCommand::LoadActiveSlot);
            }
            ViewShortcut::SelectNextSlot => {
                return self.dispatch_command(SessionCommand::SelectNextSlot);
            }
            ViewShortcut::SelectPreviousSlot => {
                return self.dispatch_command(SessionCommand::SelectPreviousSlot);
            }
            ViewShortcut::Reset => {
                return self.dispatch_command(SessionCommand::Reset);
            }
        }
        Ok(outcome)
    }

    /// Settings dialog opened natively: clear held input and take the
    /// Settings hold. View-side protocol, not a framework event.
    pub fn note_settings_opened(&mut self) -> DriverOutcome {
        let mut outcome = DriverOutcome::default();
        if !self.policy.owns(pause_owners::SETTINGS) {
            self.session.clear_input();
            set_pause_owner(self.session, self.policy, pause_owners::SETTINGS, true);
            outcome.settings_changed = true;
        }
        outcome
    }

    /// Settings dialog closed (or its native creation failed): release the
    /// Settings hold. View-side protocol, not a framework event.
    pub fn note_settings_dismissed(&mut self) -> DriverOutcome {
        let mut outcome = DriverOutcome::default();
        if self.policy.owns(pause_owners::SETTINGS) {
            set_pause_owner(self.session, self.policy, pause_owners::SETTINGS, false);
            outcome.settings_changed = true;
        }
        outcome
    }
}

/// Framework boundary entry: opaque inputs in, command outcome out.
/// Effects and settings transitions stay in [`DriverOutcome`]; the
/// [`crate::event`] handler uses [`dispatch_driver`] instead.
pub fn dispatch(
    session: &mut SessionHandle,
    policy: &mut PausePolicy,
    table: &CommandTable,
    event: SessionEvent,
) -> Result<SessionOutcome, SessionError> {
    dispatch_driver(session, policy, table, event).map(SessionOutcome::from)
}

/// Driver entry for view-side handlers needing effects and transitions.
pub fn dispatch_driver(
    session: &mut SessionHandle,
    policy: &mut PausePolicy,
    table: &CommandTable,
    event: SessionEvent,
) -> Result<DriverOutcome, SessionError> {
    SessionDriver::new(session, policy, table).dispatch(event)
}

/// Typed command entry for direct callers holding domain commands
/// (debugger drain, shell ports). Opaque activations go through [`dispatch`].
pub fn dispatch_command(
    session: &mut SessionHandle,
    policy: &mut PausePolicy,
    command: SessionCommand,
) -> Result<SessionOutcome, SessionError> {
    SessionDriver::new(session, policy, &CommandTable::default())
        .dispatch_command(command)
        .map(SessionOutcome::from)
}

/// Ownership changes are retained even without a core. The execution hold gates
/// load/apply activation before a worker can run; commands synchronize a live one.
///
/// Adoption policy lives here, next to the owner table: acquiring the first
/// hold over already-suspended content adopts an explicit user hold, so a
/// settings or modal hold never manifests as a user hold on release.
pub fn set_pause_owner(
    session: &mut SessionHandle,
    policy: &mut PausePolicy,
    owner: PauseOwner,
    active: bool,
) -> SessionCommandOutcome {
    use pause_owners::USER;
    let tick = session.tick_metrics();
    if active && !policy.paused() && tick.paused && owner != USER {
        policy.set(USER, true);
    }
    let paused = policy.set(owner, active);
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
        table: CommandTable,
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
                table: CommandTable::default(),
                loader,
                root,
            }
        }

        fn driver(&mut self) -> SessionDriver<'_> {
            SessionDriver::new(&mut self.session, &mut self.policy, &self.table)
        }

        fn dispatch_session(&mut self, event: SessionEvent) -> Result<DriverOutcome, SessionError> {
            SessionDriver::new(
                &mut self.session,
                &mut self.policy,
                &CommandTable::default(),
            )
            .dispatch(event)
        }

        fn dispatch_session_command(
            &mut self,
            command: SessionCommand,
        ) -> Result<SessionOutcome, SessionError> {
            dispatch_command(&mut self.session, &mut self.policy, command)
        }

        fn event(&mut self, event: SessionEvent) -> DriverOutcome {
            self.dispatch_session(event).unwrap()
        }

        fn command(&mut self, command: SessionCommand) -> SessionOutcome {
            self.dispatch_session_command(command).unwrap()
        }

        fn settings_opened(&mut self) -> DriverOutcome {
            self.driver().note_settings_opened()
        }

        fn settings_dismissed(&mut self) -> DriverOutcome {
            self.driver().note_settings_dismissed()
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

        fn key(&mut self, key: Key, pressed: bool) -> DriverOutcome {
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
            (ShortcutAction::OpenDebugger, DriverEffect::OpenDebugger),
            (
                ShortcutAction::ToggleFullscreen,
                DriverEffect::ToggleFullscreen,
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
        assert!(!host.settings_dismissed().settings_changed);
        assert!(host.settings_opened().settings_changed);
        assert!(!host.settings_opened().settings_changed);
        assert!(host.key(Key::F2, true).effect.is_none());
        assert!(host.settings_dismissed().settings_changed);
        assert!(!host.settings_dismissed().settings_changed);
        assert!(!host.settings_dismissed().settings_changed);
        assert!(!host.policy.paused());
        assert!(
            host.key(Key::F2, true).effect.is_some(),
            "open cleared the previous press"
        );
        assert!(
            host.settings_opened().settings_changed,
            "failure permits retry"
        );
        assert!(host.settings_dismissed().settings_changed);
    }

    #[test]
    fn shared_narrow_ports_remain_total_without_core() {
        use nerust_gui_shell::session::access::{PauseControl, ResetControl, SlotIo, SlotNav};
        let mut host = TestHost::new(false);
        host.pause();
        assert!(host.policy.owns(pause_owners::USER));
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
        assert!(host.policy.owns(pause_owners::USER));
        assert!(host.session.tick_metrics().paused);
        assert!(host.key(Key::F2, true).command.is_none());
        host.command(SessionCommand::Resume);
        assert!(!host.policy.paused());
        assert!(!host.session.tick_metrics().paused);
        host.key(Key::F2, false);
        host.key(Key::F2, true);
        assert!(host.policy.owns(pause_owners::USER));
    }

    #[test]
    fn settings_modal_focus_and_user_compose_across_real_nes_replacement_and_apply() {
        let mut host = TestHost::new(true);
        // An owner acquired with no core must gate the first load too.
        host.settings_opened();
        host.load_nes();
        assert!(host.session.tick_metrics().paused);
        assert_eq!(host.session.tick_metrics().frame_counter, 0);
        host.owner(pause_owners::MODAL, true);
        host.owner(pause_owners::FOCUS, true);
        host.load_nes();
        assert!(host.session.tick_metrics().paused);
        assert_eq!(host.session.tick_metrics().frame_counter, 0);
        let snapshot = host.session.settings_snapshot().clone();
        apply_settings(&mut host.session, &host.policy, snapshot).unwrap();
        let mut invalid = host.session.settings_snapshot().clone();
        invalid.local.audio.master_volume_percent = 101;
        assert!(apply_settings(&mut host.session, &host.policy, invalid).is_err());
        assert!(host.session.tick_metrics().paused);
        host.settings_dismissed();
        host.settings_dismissed();
        host.owner(pause_owners::FOCUS, false);
        assert!(host.session.tick_metrics().paused, "modal still holds");
        host.command(SessionCommand::Pause);
        host.owner(pause_owners::MODAL, false);
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
        host.owner(pause_owners::MODAL, true);
        host.settings_opened();
        host.settings_opened();
        assert!(
            !host.policy.owns(pause_owners::USER),
            "duplicate open never adopts its own pause"
        );
        host.settings_dismissed();
        assert!(host.policy.owns(pause_owners::MODAL));
        assert!(!host.policy.owns(pause_owners::SETTINGS));
        assert!(host.session.tick_metrics().paused);
        host.owner(pause_owners::MODAL, false);
        assert!(!host.session.tick_metrics().paused);
        assert!(host.settings_opened().settings_changed);
        assert!(host.session.tick_metrics().paused);
        host.settings_dismissed();
        assert!(!host.session.tick_metrics().paused);
    }

    #[test]
    fn observed_explicit_pause_is_adopted_but_duplicate_close_never_resumes_it() {
        let mut host = TestHost::new(true);
        host.load_nes();
        host.session.run_command(SessionCommand::Pause).unwrap();
        host.settings_opened();
        assert!(host.policy.owns(pause_owners::USER));
        host.settings_dismissed();
        assert!(host.session.tick_metrics().paused);
        host.command(SessionCommand::Resume);
        host.session.run_command(SessionCommand::Pause).unwrap();
        assert!(!host.settings_dismissed().settings_changed);
        assert!(!host.settings_dismissed().settings_changed);
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
        host.settings_opened();
        host.command(SessionCommand::Resume);
        assert!(!host.policy.owns(pause_owners::USER));
        assert!(host.session.tick_metrics().paused);
        host.settings_dismissed();
        assert!(!host.session.tick_metrics().paused);
    }
}

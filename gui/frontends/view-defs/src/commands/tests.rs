use super::*;
use nerust_gui_shell::session::commands::SessionCommand;
use nerust_persistence::{model::StateSlotSummary, sidecar::StorageLocation};
use wakui::commands::{Command, CommandRef, CommandVisitor};
use wakui::control::IdRegistry;

fn signature() -> MenuSig {
    MenuSig {
        loaded: true,
        paused: false,
        settings_open: false,
        language: nerust_settings_core::model::language::AppLanguage::English,
        active_slot: Some(42),
        slots: vec![StateSlotSummary {
            schema_version: 1,
            slot_id: 42,
            path: StorageLocation::Native("slot42".into()),
            saved_at: std::time::UNIX_EPOCH,
            has_thumbnail: false,
            emulator_version: "test".into(),
        }]
        .into(),
        shortcuts: Vec::new().into(),
    }
}

fn build() -> (CommandModel, CommandTable) {
    build_command_model(&mut IdRegistry::default(), &signature())
}

#[test]
fn every_command_round_trips_with_unique_stable_declarations() {
    use SessionCommand as S;
    let mut commands = vec![
        MenuCommand::Open,
        MenuCommand::Close,
        MenuCommand::Settings,
        MenuCommand::Debugger,
        MenuCommand::NotificationHistory,
        MenuCommand::Native(NativeRole::About),
        MenuCommand::Native(NativeRole::Quit),
    ];
    commands.extend(
        [
            S::Pause,
            S::Resume,
            S::TogglePause,
            S::Reset,
            S::CreateSlot,
            S::SaveActiveSlotOrNew,
            S::LoadActiveSlot,
            S::SelectActiveSlot(42),
            S::SaveSlot(42),
            S::LoadSlot(42),
            S::DeleteSlot(42),
            S::SelectNextSlot,
            S::SelectPreviousSlot,
        ]
        .map(MenuCommand::Session),
    );
    let (model, table) = build();
    assert_eq!(commands.len(), model.definitions.len());
    let mut names = std::collections::HashSet::new();
    let mut ids = std::collections::HashSet::new();
    for command in commands {
        let activation = table.activation(command).expect("activation");
        assert!(names.insert(command.binding().0.name()));
        assert!(ids.insert(activation.id));
        // Resolution is structural: same id plus same argument shape.
        assert_eq!(table.resolve(activation), Some(command));
        let declaration = model.get(activation.id).unwrap();
        assert!(declaration.accepts(activation));
        assert_eq!(declaration.enabled, command_enabled(command, &signature()));
        // Wrong argument shape never resolves.
        let flipped = CommandRef {
            id: activation.id,
            argument: if activation.argument.is_some() {
                None
            } else {
                Some(42)
            },
        };
        assert_eq!(table.resolve(flipped), None);
        assert!(!declaration.accepts(flipped));
        if let Some(arg) = activation.argument {
            let _ = arg;
            for value in [0, u64::MAX] {
                let resolved = table
                    .resolve(CommandRef {
                        id: activation.id,
                        argument: Some(value),
                    })
                    .unwrap();
                assert_eq!(resolved.binding(), (command.binding().0, Some(value)));
            }
        }
    }
    assert_eq!(
        DECLARATIONS
            .iter()
            .map(|(_, name, _)| *name)
            .collect::<Vec<_>>(),
        [
            "open",
            "close",
            "settings",
            "debugger",
            "history",
            "quit",
            "about",
            "pause",
            "resume",
            "reset",
            "state-create",
            "state-save-active",
            "state-load-active",
            "state-select-slot",
            "state-save-slot",
            "state-load-slot",
            "state-delete-slot",
            "toggle-pause",
            "state-select-next",
            "state-select-previous",
        ]
    );
}

#[test]
fn declarations_gate_slot_parameters_but_direct_commands_validate_the_slot() {
    let mut sig = signature();
    let slots = [
        MenuCommandId::SelectActiveSlot,
        MenuCommandId::SaveSlot,
        MenuCommandId::LoadSlot,
        MenuCommandId::DeleteSlot,
    ];
    assert!(!command_enabled(
        MenuCommand::Session(SessionCommand::SaveSlot(99)),
        &sig
    ));
    for (loaded, settings_open, nonempty) in [
        (true, false, true),
        (true, false, false),
        (false, false, true),
        (true, true, true),
    ] {
        sig.loaded = loaded;
        sig.settings_open = settings_open;
        sig.slots = if nonempty {
            signature().slots
        } else {
            Vec::new().into()
        };
        let mut ids = IdRegistry::default();
        let (model, table) = build_command_model(&mut ids, &sig);
        for declaration in &model.definitions {
            let key = table.key(declaration.id).expect("domain key");
            if slots.contains(&key) {
                assert_eq!(declaration.enabled, loaded && !settings_open && nonempty);
            }
        }
        if settings_open {
            let quit = table
                .activation(MenuCommand::Native(NativeRole::Quit))
                .unwrap();
            for declaration in &model.definitions {
                assert_eq!(declaration.enabled, declaration.id == quit.id);
            }
        }
    }
}

#[test]
fn command_visitor_is_object_safe() {
    struct Count(usize);
    impl CommandVisitor for Count {
        fn command(&mut self, _: &Command) {
            self.0 += 1;
        }
    }
    let (model, _) = build();
    let mut count = Count(0);
    let visitor: &mut dyn CommandVisitor = &mut count;
    model.visit(visitor);
    assert_eq!(count.0, model.definitions.len());
}

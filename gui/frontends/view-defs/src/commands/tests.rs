use super::*;
use nerust_persistence::{model::StateSlotSummary, sidecar::StorageLocation};

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
    let model = CommandModel::build(&signature());
    assert_eq!(commands.len(), model.definitions.len());
    let mut names = std::collections::HashSet::new();
    let mut ids = std::collections::HashSet::new();
    for command in commands {
        let (id, arg) = command.binding();
        assert!(names.insert(id.name()));
        assert!(ids.insert(id));
        let descriptor = model.get(id).unwrap();
        assert_eq!(descriptor.bind(arg), Some(command));
        assert_eq!(descriptor.enabled, command.enabled(&signature()));
        assert!(
            descriptor
                .bind(if arg.is_some() { None } else { Some(42) })
                .is_none()
        );
        let mut malformed = *descriptor;
        malformed.parameter = if arg.is_some() {
            CommandParameter::None
        } else {
            CommandParameter::SlotId
        };
        assert!(malformed.bind(arg).is_none());
        if arg.is_some() {
            for value in [0, u64::MAX] {
                assert_eq!(
                    descriptor.bind(Some(value)).unwrap().binding(),
                    (id, Some(value))
                );
            }
        }
    }
    assert_eq!(
        model
            .definitions
            .iter()
            .map(|d| d.id.name())
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
        CommandId::SelectActiveSlot,
        CommandId::SaveSlot,
        CommandId::LoadSlot,
        CommandId::DeleteSlot,
    ];
    assert!(!MenuCommand::Session(SessionCommand::SaveSlot(99)).enabled(&sig));
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
        let model = CommandModel::build(&sig);
        for descriptor in &model.definitions {
            if slots.contains(&descriptor.id) {
                assert_eq!(descriptor.enabled, loaded && !settings_open && nonempty);
            }
            if settings_open {
                assert_eq!(descriptor.enabled, descriptor.id == CommandId::Quit);
            }
        }
    }
}

#[test]
fn command_visitor_is_object_safe() {
    struct Count(usize);
    impl CommandVisitor for Count {
        fn command(&mut self, _: &CommandDescriptor) {
            self.0 += 1;
        }
    }
    let model = CommandModel::build(&signature());
    let mut count = Count(0);
    let visitor: &mut dyn CommandVisitor = &mut count;
    model.visit(visitor);
    assert_eq!(count.0, model.definitions.len());
}

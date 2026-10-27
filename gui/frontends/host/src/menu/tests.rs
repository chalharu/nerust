use super::*;
use nerust_persistence::{model::StateSlotSummary, sidecar::StorageLocation};
use nerust_settings_core::model::{
    input::{ShortcutBinding, default_shortcut_bindings},
    language::AppLanguage,
};

const CAPABILITIES: MenuCapabilities = MenuCapabilities {
    about: true,
    dialog_ellipsis: false,
};

fn sig() -> MenuSig {
    MenuSig {
        loaded: false,
        paused: false,
        settings_open: false,
        language: AppLanguage::English,
        active_slot: None,
        slots: Vec::new().into(),
        shortcuts: default_shortcut_bindings().into(),
    }
}

fn slot(id: u64) -> StateSlotSummary {
    StateSlotSummary {
        schema_version: 1,
        slot_id: id,
        path: StorageLocation::Native(format!("slot{id}").into()),
        saved_at: std::time::UNIX_EPOCH,
        has_thumbnail: false,
        emulator_version: "test".into(),
    }
}

fn items(menu: &Submenu) -> Vec<&MenuItem> {
    menu.children
        .iter()
        .flat_map(|child| match child {
            MenuEntry::Item(item) => vec![item],
            MenuEntry::Submenu(menu) => items(menu),
        })
        .collect()
}

fn command_item(model: &MenuModel, command: MenuCommand) -> &MenuItem {
    model
        .menus
        .iter()
        .flat_map(items)
        .find(|item| item.command == Some(command))
        .unwrap()
}

#[test]
fn semantic_tree_owns_order_roles_and_disabled_placeholders() {
    let model = MenuModel::build(&sig(), CAPABILITIES);
    assert_eq!(
        model.menus.iter().map(|menu| menu.id).collect::<Vec<_>>(),
        [MenuId::File, MenuId::Emulation, MenuId::Help]
    );
    assert_eq!(
        items(&model.menus[0])
            .iter()
            .map(|item| item.command.unwrap())
            .collect::<Vec<_>>(),
        [
            MenuCommand::Open,
            MenuCommand::Close,
            MenuCommand::Settings,
            MenuCommand::Native(NativeRole::Quit),
        ]
    );
    let states = find_menu(&model.menus, MenuId::States).unwrap();
    assert_eq!(states.children.len(), 7);
    for id in [
        MenuId::SelectSlot,
        MenuId::SaveSlot,
        MenuId::LoadSlot,
        MenuId::DeleteSlot,
    ] {
        let menu = find_menu(&model.menus, id).unwrap();
        assert!(!menu.enabled);
        let entries = items(menu);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].command, None);
        assert!(!entries[0].enabled);
        assert_eq!(
            entries[0].label,
            text(AppLanguage::English, UiText::NoSaveSlots)
        );
    }
    let tao = MenuModel::build(
        &sig(),
        MenuCapabilities {
            about: false,
            dialog_ellipsis: true,
        },
    );
    assert!(find_menu(&tao.menus, MenuId::Help).is_none());
    assert!(command_item(&tao, MenuCommand::Open).label.ends_with("..."));
}

#[test]
fn translation_shortcut_and_slot_labels_have_typed_commands() {
    let mut sig = sig();
    sig.loaded = true;
    sig.slots = vec![slot(42)].into();
    sig.active_slot = Some(42);
    let model = MenuModel::build(&sig, CAPABILITIES);
    assert_eq!(
        command_item(&model, MenuCommand::Debugger).label,
        "Open Debugger (F4)"
    );
    assert_eq!(
        command_item(
            &model,
            MenuCommand::Session(SessionCommand::SaveActiveSlotOrNew)
        )
        .label,
        "Save Active Slot (F5)"
    );
    for command in [
        SessionCommand::SelectActiveSlot(42),
        SessionCommand::SaveSlot(42),
        SessionCommand::LoadSlot(42),
        SessionCommand::DeleteSlot(42),
    ] {
        let item = command_item(&model, MenuCommand::Session(command));
        assert_eq!(item.label, slot_label(&slot(42), Some(42)));
        assert!(item.enabled);
    }
    sig.language = AppLanguage::Japanese;
    sig.shortcuts = vec![ShortcutBinding {
        action: ShortcutAction::OpenDebugger,
        key: None,
    }]
    .into();
    let japanese = MenuModel::build(&sig, CAPABILITIES);
    assert_eq!(japanese.menus[0].label, "ファイル");
    assert_eq!(
        command_item(&japanese, MenuCommand::Debugger).label,
        text(sig.language, UiText::OpenDebugger)
    );
}

#[test]
fn coreless_debugger_history_and_settings_guard_including_quit() {
    let mut sig = sig();
    sig.slots = vec![slot(1)].into();
    sig.active_slot = Some(1);
    let model = MenuModel::build(&sig, CAPABILITIES);
    for item in model.menus.iter().flat_map(items) {
        assert_eq!(
            item.enabled,
            matches!(
                item.command,
                Some(
                    MenuCommand::Open
                        | MenuCommand::Settings
                        | MenuCommand::Debugger
                        | MenuCommand::NotificationHistory
                        | MenuCommand::Native(_)
                )
            )
        );
    }
    sig.loaded = true;
    sig.settings_open = true;
    let model = MenuModel::build(&sig, CAPABILITIES);
    for item in model.menus.iter().flat_map(items) {
        assert_eq!(
            item.enabled,
            item.command == Some(MenuCommand::Native(NativeRole::Quit))
        );
    }
    assert!(command_item(&model, MenuCommand::Native(NativeRole::Quit)).enabled);
}

#[test]
fn pause_and_active_slot_gates_preserve_save_without_existing_slot() {
    let mut sig = sig();
    sig.loaded = true;
    let model = MenuModel::build(&sig, CAPABILITIES);
    assert!(command_item(&model, MenuCommand::Session(SessionCommand::Pause)).enabled);
    assert!(!command_item(&model, MenuCommand::Session(SessionCommand::Resume)).enabled);
    assert!(
        command_item(
            &model,
            MenuCommand::Session(SessionCommand::SaveActiveSlotOrNew)
        )
        .enabled
    );
    assert!(!command_item(&model, MenuCommand::Session(SessionCommand::LoadActiveSlot)).enabled);
    sig.paused = true;
    sig.active_slot = Some(1);
    sig.slots = vec![slot(1)].into();
    let model = MenuModel::build(&sig, CAPABILITIES);
    assert!(!command_item(&model, MenuCommand::Session(SessionCommand::Pause)).enabled);
    assert!(command_item(&model, MenuCommand::Session(SessionCommand::Resume)).enabled);
    assert!(command_item(&model, MenuCommand::Session(SessionCommand::LoadActiveSlot)).enabled);
}

#[derive(Default)]
struct Recorder {
    stack: Vec<MenuId>,
    preorder: Vec<MenuId>,
    rebuilds: Vec<MenuId>,
    root_rebuild: bool,
    commands: Vec<Option<MenuCommand>>,
}

impl MenuVisitor for Recorder {
    fn begin_model(&mut self, rebuild: bool) {
        self.root_rebuild = rebuild;
    }
    fn begin_menu(&mut self, menu: &Submenu, rebuild: bool) {
        self.stack.push(menu.id);
        self.preorder.push(menu.id);
        if rebuild {
            self.rebuilds.push(menu.id);
        }
    }
    fn item(&mut self, item: &MenuItem) {
        assert!(!self.stack.is_empty());
        self.commands.push(item.command);
    }
    fn end_menu(&mut self, id: MenuId) {
        assert_eq!(self.stack.pop(), Some(id));
    }
    fn end_model(&mut self) {
        assert!(self.stack.is_empty());
    }
}

#[test]
fn visitor_balanced_preorder_and_incremental_enable_vs_structure() {
    let mut sig = sig();
    let first = MenuModel::build(&sig, CAPABILITIES);
    let mut record = Recorder::default();
    first.visit(None, &mut record);
    assert_eq!(
        record.preorder,
        [
            MenuId::File,
            MenuId::Emulation,
            MenuId::States,
            MenuId::SelectSlot,
            MenuId::SaveSlot,
            MenuId::LoadSlot,
            MenuId::DeleteSlot,
            MenuId::Help
        ]
    );
    assert_eq!(record.rebuilds, record.preorder);
    assert!(record.root_rebuild);
    sig.loaded = true;
    sig.settings_open = true;
    let next = MenuModel::build(&sig, CAPABILITIES);
    let mut record = Recorder::default();
    next.visit(Some(&first), &mut record);
    assert!(record.rebuilds.is_empty());
    assert!(!record.root_rebuild);
    sig.slots = vec![slot(1)].into();
    let slots = MenuModel::build(&sig, CAPABILITIES);
    let mut record = Recorder::default();
    slots.visit(Some(&next), &mut record);
    assert_eq!(
        record.rebuilds,
        [
            MenuId::SelectSlot,
            MenuId::SaveSlot,
            MenuId::LoadSlot,
            MenuId::DeleteSlot
        ]
    );
    assert!(!record.root_rebuild);
    sig.slots = sig.slots.to_vec().into();
    let reallocated = MenuModel::build(&sig, CAPABILITIES);
    assert_eq!(reallocated, slots);
    sig.shortcuts = Vec::new().into();
    let mut record = Recorder::default();
    MenuModel::build(&sig, CAPABILITIES).visit(Some(&slots), &mut record);
    assert_eq!(record.rebuilds, [MenuId::Emulation, MenuId::States]);
    assert!(!record.root_rebuild);
}

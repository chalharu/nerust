use super::menu_id;
use super::*;
use crate::commands::{CommandTable, MenuCommand, NativeRole};
use nerust_gui_shell::session::commands::SessionCommand;
use nerust_persistence::{model::StateSlotSummary, sidecar::StorageLocation};
use nerust_settings_core::model::{
    input::{ShortcutBinding, default_shortcut_bindings},
    language::AppLanguage,
};
use wakui::chrome::menu::{CommandVisitor, MenuId, MenuModel, MenuVisitor, find_menu};
use wakui::input::commands::CommandRef;
use wakui::scene::control::IdRegistry;

const CAPABILITIES: MenuCapabilities = MenuCapabilities {
    about: true,
    dialog_ellipsis: false,
    application_menu: false,
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

fn built(sig: &MenuSig, capabilities: MenuCapabilities) -> (MenuModel, CommandTable) {
    build_menubar(&mut IdRegistry::default(), sig, capabilities)
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

fn activation(table: &CommandTable, command: MenuCommand) -> CommandRef {
    table.activation(command).expect("activation")
}

fn command_item<'a>(
    model: &'a MenuModel,
    table: &CommandTable,
    command: MenuCommand,
) -> &'a MenuItem {
    let activation = activation(table, command);
    model
        .menus
        .iter()
        .flat_map(items)
        .find(|item| item.command == Some(activation))
        .unwrap()
}

#[test]
fn semantic_tree_owns_order_roles_and_disabled_placeholders() {
    let (model, table) = built(&sig(), CAPABILITIES);
    assert_eq!(
        model.menus.iter().map(|menu| menu.id).collect::<Vec<_>>(),
        [menu_id::FILE, menu_id::EMULATION, menu_id::HELP]
    );
    assert_eq!(
        items(&model.menus[0])
            .iter()
            .map(|item| item.command.unwrap())
            .collect::<Vec<_>>(),
        [
            activation(&table, MenuCommand::Open),
            activation(&table, MenuCommand::Close),
            activation(&table, MenuCommand::Settings),
            activation(&table, MenuCommand::Native(NativeRole::Quit)),
        ]
    );
    let states = find_menu(&model.menus, menu_id::STATES).unwrap();
    assert_eq!(states.children.len(), 7);
    for id in [
        menu_id::SELECT_SLOT,
        menu_id::SAVE_SLOT,
        menu_id::LOAD_SLOT,
        menu_id::DELETE_SLOT,
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
    let (tao, tao_table) = built(
        &sig(),
        MenuCapabilities {
            about: false,
            dialog_ellipsis: true,
            application_menu: false,
        },
    );
    assert!(find_menu(&tao.menus, menu_id::HELP).is_none());
    assert!(
        command_item(&tao, &tao_table, MenuCommand::Open)
            .label
            .ends_with("...")
    );
}

#[test]
fn translation_shortcut_and_slot_labels_have_typed_commands() {
    let mut sig = sig();
    sig.loaded = true;
    sig.slots = vec![slot(42)].into();
    sig.active_slot = Some(42);
    let (model, table) = built(&sig, CAPABILITIES);
    assert_eq!(
        command_item(&model, &table, MenuCommand::Debugger).label,
        "Open Debugger (F4)"
    );
    assert_eq!(
        command_item(
            &model,
            &table,
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
        let item = command_item(&model, &table, MenuCommand::Session(command));
        assert_eq!(item.label, slot_label(&slot(42), Some(42)));
        assert!(item.enabled);
    }
    sig.language = AppLanguage::Japanese;
    sig.shortcuts = vec![ShortcutBinding {
        action: ShortcutAction::OpenDebugger,
        key: None,
    }]
    .into();
    let (japanese, japanese_table) = built(&sig, CAPABILITIES);
    assert_eq!(japanese.menus[0].label, "ファイル");
    assert_eq!(
        command_item(&japanese, &japanese_table, MenuCommand::Debugger).label,
        text(sig.language, UiText::OpenDebugger)
    );
}

#[test]
fn coreless_debugger_history_and_settings_guard_including_quit() {
    let mut sig = sig();
    sig.slots = vec![slot(1)].into();
    sig.active_slot = Some(1);
    let (model, table) = built(&sig, CAPABILITIES);
    let unit = |command| activation(&table, command).argument.is_none();
    for item in model.menus.iter().flat_map(items) {
        let Some(activation) = item.command else {
            continue;
        };
        let resolved = table.resolve(activation).unwrap();
        assert_eq!(
            item.enabled,
            unit(resolved)
                && matches!(
                    resolved,
                    MenuCommand::Open
                        | MenuCommand::Settings
                        | MenuCommand::Debugger
                        | MenuCommand::NotificationHistory
                        | MenuCommand::Native(_)
                )
        );
    }
    sig.loaded = true;
    sig.settings_open = true;
    let (model, table) = built(&sig, CAPABILITIES);
    for item in model.menus.iter().flat_map(items) {
        assert_eq!(
            item.enabled,
            item.command == Some(activation(&table, MenuCommand::Native(NativeRole::Quit)))
        );
    }
    assert!(command_item(&model, &table, MenuCommand::Native(NativeRole::Quit)).enabled);
}

#[test]
fn pause_and_active_slot_gates_preserve_save_without_existing_slot() {
    let mut sig = sig();
    sig.loaded = true;
    let (model, table) = built(&sig, CAPABILITIES);
    assert!(command_item(&model, &table, MenuCommand::Session(SessionCommand::Pause)).enabled);
    assert!(!command_item(&model, &table, MenuCommand::Session(SessionCommand::Resume)).enabled);
    assert!(
        command_item(
            &model,
            &table,
            MenuCommand::Session(SessionCommand::SaveActiveSlotOrNew)
        )
        .enabled
    );
    assert!(
        !command_item(
            &model,
            &table,
            MenuCommand::Session(SessionCommand::LoadActiveSlot)
        )
        .enabled
    );
    sig.paused = true;
    sig.active_slot = Some(1);
    sig.slots = vec![slot(1)].into();
    let (model, table) = built(&sig, CAPABILITIES);
    assert!(!command_item(&model, &table, MenuCommand::Session(SessionCommand::Pause)).enabled);
    assert!(command_item(&model, &table, MenuCommand::Session(SessionCommand::Resume)).enabled);
    assert!(
        command_item(
            &model,
            &table,
            MenuCommand::Session(SessionCommand::LoadActiveSlot)
        )
        .enabled
    );
}

#[derive(Default)]
struct Recorder {
    root_open: bool,
    stack: Vec<MenuId>,
    preorder: Vec<MenuId>,
    rebuilds: Vec<MenuId>,
    root_rebuild: bool,
    commands: Vec<Option<CommandRef>>,
    declarations: Vec<wakui::input::commands::CommandId>,
}

impl MenuVisitor for Recorder {
    fn begin_menubar(&mut self, _: &Menubar, rebuild: bool) {
        assert!(!self.root_open);
        self.root_open = true;
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
    fn end_menubar(&mut self) {
        assert!(self.root_open);
        assert!(self.stack.is_empty());
        self.root_open = false;
    }
}

impl CommandVisitor for Recorder {
    fn command(&mut self, command: &wakui::input::commands::Command) {
        assert!(self.root_open, "root precedes declarations");
        assert!(self.stack.is_empty(), "declarations precede menus");
        // Opaque framework ids only; the resolution table maps them back.
        self.declarations.push(command.id);
    }
}

#[test]
fn visitor_balanced_preorder_and_incremental_enable_vs_structure() {
    let mut signature = sig();
    let (first, _) = built(&signature, CAPABILITIES);
    let mut record = Recorder::default();
    first.visit(None, &mut record);
    assert_eq!(record.declarations.len(), first.commands.definitions.len());
    assert_eq!(
        record.preorder,
        [
            menu_id::FILE,
            menu_id::EMULATION,
            menu_id::STATES,
            menu_id::SELECT_SLOT,
            menu_id::SAVE_SLOT,
            menu_id::LOAD_SLOT,
            menu_id::DELETE_SLOT,
            menu_id::HELP
        ]
    );
    assert_eq!(record.rebuilds, record.preorder);
    assert!(record.root_rebuild);
    signature.loaded = true;
    signature.settings_open = true;
    // Same registry: opaque ids stay stable, so only enablement changes.
    let mut ids = IdRegistry::default();
    let (next, _) = build_menubar(&mut ids, &signature, CAPABILITIES);
    let (first, _) = build_menubar(&mut ids, &sig(), CAPABILITIES);
    let mut record = Recorder::default();
    next.visit(Some(&first), &mut record);
    assert!(record.rebuilds.is_empty());
    assert!(!record.root_rebuild);
    signature.slots = vec![slot(1)].into();
    let (slots, _) = build_menubar(&mut ids, &signature, CAPABILITIES);
    let mut record = Recorder::default();
    slots.visit(Some(&next), &mut record);
    assert_eq!(
        record.rebuilds,
        [
            menu_id::SELECT_SLOT,
            menu_id::SAVE_SLOT,
            menu_id::LOAD_SLOT,
            menu_id::DELETE_SLOT
        ]
    );
    assert!(!record.root_rebuild);
    signature.slots = signature.slots.to_vec().into();
    let (reallocated, _) = build_menubar(&mut ids, &signature, CAPABILITIES);
    assert_eq!(reallocated, slots);
    signature.shortcuts = Vec::new().into();
    let mut record = Recorder::default();
    build_menubar(&mut ids, &signature, CAPABILITIES)
        .0
        .visit(Some(&slots), &mut record);
    assert_eq!(record.rebuilds, [menu_id::EMULATION, menu_id::STATES]);
    assert!(!record.root_rebuild);
}

#[test]
fn application_menu_is_host_owned_and_part_of_root_diff_through_dyn_visitor() {
    use NativeApplicationEntry::{Role, Separator};
    use NativeApplicationRole as R;
    // One registry: opaque command ids stay stable across both builds, so
    // only the application-menu addition rebuilds the root.
    let mut ids = IdRegistry::default();
    let (first, _) = build_menubar(&mut ids, &sig(), CAPABILITIES);
    assert!(first.application_menu.is_none());
    let (next, _) = build_menubar(
        &mut ids,
        &sig(),
        MenuCapabilities {
            application_menu: true,
            ..CAPABILITIES
        },
    );
    let app = next.application_menu.as_ref().unwrap();
    assert_eq!(app.label, "App");
    assert_eq!(
        app.entries,
        [
            Role(R::About),
            Separator,
            Role(R::Services),
            Separator,
            Role(R::Hide),
            Role(R::HideOthers),
            Role(R::ShowAll),
            Separator,
            Role(R::Quit),
        ]
    );
    let mut record = Recorder::default();
    let visitor: &mut dyn MenuVisitor = &mut record;
    next.visit(Some(&first), visitor);
    assert!(record.root_rebuild);
    assert!(record.rebuilds.is_empty());
    let mut record = Recorder::default();
    next.visit(Some(&next), &mut record);
    assert!(!record.root_rebuild);
    let mut changed = next.clone();
    changed.application_menu.as_mut().unwrap().label = "Changed".into();
    changed.visit(Some(&next), &mut record);
    assert!(record.root_rebuild);
    let mut changed = next.clone();
    changed.application_menu.as_mut().unwrap().entries.pop();
    changed.visit(Some(&next), &mut record);
    assert!(record.root_rebuild);
    first.visit(Some(&next), &mut record);
    assert!(
        record.root_rebuild,
        "removing the application menu rebuilds root"
    );
}

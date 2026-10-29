//! Menu builders over the signature: titles, signature assembly, models.
//! Schema (ids, models, visitors) lives in the framework
//! (`wakui::menu`).

use wakui::chrome::menu::{
    FileDialogTitles, MenuCapabilities, MenuEntry, MenuItem, Menubar, NativeApplicationEntry,
    NativeApplicationMenu, NativeApplicationRole, Submenu,
};
use wakui::scene::control::IdRegistry;

/// View-side menu skeleton: what each opaque [`MenuId`] means. The framework
/// only tracks the ids for native reuse; the File/Emulation/States shape,
/// slot submenus, and Help live here.
pub mod menu_id {
    use wakui::chrome::menu::MenuId;

    pub const FILE: MenuId = MenuId::from_raw(0);
    pub const EMULATION: MenuId = MenuId::from_raw(1);
    pub const STATES: MenuId = MenuId::from_raw(2);
    pub const SELECT_SLOT: MenuId = MenuId::from_raw(3);
    pub const SAVE_SLOT: MenuId = MenuId::from_raw(4);
    pub const LOAD_SLOT: MenuId = MenuId::from_raw(5);
    pub const DELETE_SLOT: MenuId = MenuId::from_raw(6);
    pub const HELP: MenuId = MenuId::from_raw(7);
}

use crate::commands::{
    MenuCommand, MenuCommandId, NativeRole, build_command_model, command_enabled,
};
use nerust_gui_shell::session::commands::SessionCommand;

use nerust_gui_runtime::{menu_labels::shortcut_hint, slots::slot_label};
use nerust_settings_core::{
    i18n::{UiText, text},
    model::input::ShortcutAction,
};

/// Dialog titles in the view language. ROM wording is view-side data;
/// the framework only transports the strings.
pub fn dialog_titles(
    language: nerust_settings_core::model::language::AppLanguage,
) -> FileDialogTitles {
    use nerust_settings_core::i18n::{UiText, text as ui_text};
    FileDialogTitles {
        open: ui_text(language, UiText::Open).to_string(),
        error: ui_text(language, UiText::Error).to_string(),
        load_failed: ui_text(language, UiText::RomLoadFailed).to_string(),
    }
}

/// Menu chrome signature: the change-gate inputs a menubar build observes.
/// Produced by the host from the frame context; view code only reads it.
#[derive(Debug, Clone)]
pub struct MenuSig {
    pub loaded: bool,
    pub paused: bool,
    pub settings_open: bool,
    pub language: nerust_settings_core::model::language::AppLanguage,
    pub active_slot: Option<u64>,
    pub slots: std::sync::Arc<[nerust_persistence::model::StateSlotSummary]>,
    pub shortcuts: std::sync::Arc<[nerust_settings_core::model::input::ShortcutBinding]>,
}

/// Build the generic menubar plus its resolution table. The caller keeps
/// `ids` across frames so opaque command ids stay stable while labels and
/// enablement refresh, mirroring settings stable-id minting.
pub fn build_menubar(
    ids: &mut IdRegistry<MenuCommandId>,
    sig: &MenuSig,
    capabilities: MenuCapabilities,
) -> (Menubar, crate::commands::CommandTable) {
    use MenuCommand as C;
    use SessionCommand as S;
    let (commands, table) = build_command_model(ids, sig);
    let label = |key| text(sig.language, key).to_owned();
    // Opaque activation for a domain command: id from the table, argument
    // from the typed value. Flavors forward this without interpretation.
    let activation = |command: C| table.activation(command).expect("command activation");
    let item = |command: C, key: UiText| {
        let label = label(key);
        let enabled = command_enabled(command, sig);
        MenuItem {
            command: Some(activation(command)),
            label,
            enabled,
        }
    };
    let hinted = |command, key, shortcut| {
        let mut entry = item(command, key);
        entry.label = shortcut_hint(&entry.label, &sig.shortcuts, shortcut);
        MenuEntry::Item(entry)
    };
    let dialog = |command, key| {
        let mut entry = item(command, key);
        if capabilities.dialog_ellipsis {
            entry.label.push_str("...");
        }
        MenuEntry::Item(entry)
    };
    let submenu = |id, key, enabled, children| Submenu {
        id,
        label: label(key),
        enabled,
        children,
    };
    let mut states = vec![
        MenuEntry::Item(item(C::Session(S::CreateSlot), UiText::CreateSaveSlot)),
        hinted(
            C::Session(S::SaveActiveSlotOrNew),
            UiText::SaveActiveSlot,
            ShortcutAction::SaveActiveSlot,
        ),
        hinted(
            C::Session(S::LoadActiveSlot),
            UiText::LoadActiveSlot,
            ShortcutAction::LoadActiveSlot,
        ),
    ];
    for (id, key, command) in [
        (
            menu_id::SELECT_SLOT,
            UiText::SelectActiveSlot,
            S::SelectActiveSlot as fn(u64) -> S,
        ),
        (menu_id::SAVE_SLOT, UiText::SaveSlot, S::SaveSlot),
        (menu_id::LOAD_SLOT, UiText::LoadSlot, S::LoadSlot),
        (menu_id::DELETE_SLOT, UiText::DeleteSlot, S::DeleteSlot),
    ] {
        let template_opaque = table
            .activation(C::Session(command(0)))
            .map(|activation| activation.id)
            .expect("command activation");
        let enabled = commands
            .get(template_opaque)
            .expect("slot command declaration")
            .enabled;
        let children = if sig.slots.is_empty() {
            vec![MenuEntry::Item(MenuItem {
                command: None,
                label: label(UiText::NoSaveSlots),
                enabled: false,
            })]
        } else {
            sig.slots
                .iter()
                .map(|slot| {
                    let command = C::Session(command(slot.slot_id));
                    MenuEntry::Item(MenuItem {
                        command: Some(activation(command)),
                        label: slot_label(slot, sig.active_slot),
                        // This entry comes directly from the current slots;
                        // do not rescan the entire list for every item.
                        enabled,
                    })
                })
                .collect()
        };
        states.push(MenuEntry::Submenu(submenu(id, key, enabled, children)));
    }
    let mut menus = vec![
        submenu(
            menu_id::FILE,
            UiText::File,
            true,
            vec![
                dialog(C::Open, UiText::Open),
                MenuEntry::Item(item(C::Close, UiText::Close)),
                dialog(C::Settings, UiText::Settings),
                MenuEntry::Item(item(C::Native(NativeRole::Quit), UiText::Quit)),
            ],
        ),
        submenu(
            menu_id::EMULATION,
            UiText::Emulation,
            true,
            vec![
                MenuEntry::Item(item(C::Session(S::Pause), UiText::Pause)),
                MenuEntry::Item(item(C::Session(S::Resume), UiText::Resume)),
                MenuEntry::Item(item(C::Session(S::Reset), UiText::Reset)),
                MenuEntry::Submenu(submenu(menu_id::STATES, UiText::SaveStates, true, states)),
                hinted(
                    C::Debugger,
                    UiText::OpenDebugger,
                    ShortcutAction::OpenDebugger,
                ),
                MenuEntry::Item(item(C::NotificationHistory, UiText::NotificationHistory)),
            ],
        ),
    ];
    if capabilities.about {
        menus.push(submenu(
            menu_id::HELP,
            UiText::Help,
            true,
            vec![MenuEntry::Item(item(
                C::Native(NativeRole::About),
                UiText::About,
            ))],
        ));
    }
    let application_menu = capabilities.application_menu.then(|| {
        use NativeApplicationEntry::{Role, Separator};
        use NativeApplicationRole as R;
        NativeApplicationMenu {
            label: "App".into(),
            entries: vec![
                Role(R::About),
                Separator,
                Role(R::Services),
                Separator,
                Role(R::Hide),
                Role(R::HideOthers),
                Role(R::ShowAll),
                Separator,
                Role(R::Quit),
            ],
        }
    });
    (
        Menubar {
            commands,
            menus,
            application_menu,
        },
        table,
    )
}

#[cfg(test)]
mod tests;

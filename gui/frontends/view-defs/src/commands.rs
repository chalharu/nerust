//! Command model builders over the menu signature (view side).
//!
//! Domain vocabulary (`MenuCommand`, slot templates, enablement) lives here,
//! not in the framework. `wakui::commands` only transports opaque ids plus an
//! argument shape (`UnitSpec`/`IndexedSpec`); this module mints stable opaque
//! ids from domain keys, builds the generic model, and resolves activations
//! back to typed commands, like settings resolves `ControlId` via
//! `BindingTable`.

use nerust_gui_shell::session::commands::SessionCommand;
use wakui::commands::{
    Command, CommandId, CommandModel, CommandRef, CommandSpec, IndexedSpec, UnitSpec,
};
use wakui::control::IdRegistry;

use crate::menu::MenuSig;

/// Native host verbs. Framework transports them opaquely; meaning resolves here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeRole {
    About,
    Quit,
}

/// View-side menu vocabulary. Flavors never name this type; they forward
/// opaque [`CommandRef`]s and this module resolves them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuCommand {
    Open,
    Close,
    Settings,
    Debugger,
    NotificationHistory,
    Session(SessionCommand),
    Native(NativeRole),
}

/// Stable domain key for one declaration. Opaque framework ids are minted
/// from these, so rebuilds keep identities while the model stays generic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MenuCommandId {
    Open,
    Close,
    Settings,
    Debugger,
    NotificationHistory,
    Quit,
    About,
    Pause,
    Resume,
    Reset,
    CreateSlot,
    SaveActiveSlotOrNew,
    LoadActiveSlot,
    SelectActiveSlot,
    SaveSlot,
    LoadSlot,
    DeleteSlot,
    TogglePause,
    SelectNextSlot,
    SelectPreviousSlot,
}

// One declaration table supplies names, order, and templates.
// Slot commands use zero as a template, never as a default argument.
pub const DECLARATIONS: &[(MenuCommandId, &str, MenuCommand)] = {
    use MenuCommand::{Native, Session};
    use SessionCommand as S;
    &[
        (MenuCommandId::Open, "open", MenuCommand::Open),
        (MenuCommandId::Close, "close", MenuCommand::Close),
        (MenuCommandId::Settings, "settings", MenuCommand::Settings),
        (MenuCommandId::Debugger, "debugger", MenuCommand::Debugger),
        (
            MenuCommandId::NotificationHistory,
            "history",
            MenuCommand::NotificationHistory,
        ),
        (MenuCommandId::Quit, "quit", Native(NativeRole::Quit)),
        (MenuCommandId::About, "about", Native(NativeRole::About)),
        (MenuCommandId::Pause, "pause", Session(S::Pause)),
        (MenuCommandId::Resume, "resume", Session(S::Resume)),
        (MenuCommandId::Reset, "reset", Session(S::Reset)),
        (
            MenuCommandId::CreateSlot,
            "state-create",
            Session(S::CreateSlot),
        ),
        (
            MenuCommandId::SaveActiveSlotOrNew,
            "state-save-active",
            Session(S::SaveActiveSlotOrNew),
        ),
        (
            MenuCommandId::LoadActiveSlot,
            "state-load-active",
            Session(S::LoadActiveSlot),
        ),
        (
            MenuCommandId::SelectActiveSlot,
            "state-select-slot",
            Session(S::SelectActiveSlot(0)),
        ),
        (
            MenuCommandId::SaveSlot,
            "state-save-slot",
            Session(S::SaveSlot(0)),
        ),
        (
            MenuCommandId::LoadSlot,
            "state-load-slot",
            Session(S::LoadSlot(0)),
        ),
        (
            MenuCommandId::DeleteSlot,
            "state-delete-slot",
            Session(S::DeleteSlot(0)),
        ),
        (
            MenuCommandId::TogglePause,
            "toggle-pause",
            Session(S::TogglePause),
        ),
        (
            MenuCommandId::SelectNextSlot,
            "state-select-next",
            Session(S::SelectNextSlot),
        ),
        (
            MenuCommandId::SelectPreviousSlot,
            "state-select-previous",
            Session(S::SelectPreviousSlot),
        ),
    ]
};

impl MenuCommandId {
    fn declaration(self) -> &'static (Self, &'static str, MenuCommand) {
        DECLARATIONS
            .iter()
            .find(|(id, _, _)| *id == self)
            .expect("command declaration")
    }

    pub fn name(self) -> &'static str {
        self.declaration().1
    }

    fn template(self) -> MenuCommand {
        self.declaration().2
    }

    fn is_indexed(self) -> bool {
        matches!(
            self.template(),
            MenuCommand::Session(
                SessionCommand::SelectActiveSlot(_)
                    | SessionCommand::SaveSlot(_)
                    | SessionCommand::LoadSlot(_)
                    | SessionCommand::DeleteSlot(_)
            )
        )
    }
}

/// View-side resolution table: opaque framework id plus the domain template.
/// Mirrors settings `BindingTable`: flavors see only ids, meaning resolves here.
#[derive(Debug, Clone, Default)]
pub struct CommandTable(Vec<(CommandId, MenuCommandId)>);

impl CommandTable {
    fn push(&mut self, id: CommandId, key: MenuCommandId) {
        self.0.push((id, key));
    }

    fn key(&self, id: CommandId) -> Option<MenuCommandId> {
        self.0
            .iter()
            .find(|(entry, _)| *entry == id)
            .map(|(_, key)| *key)
    }

    /// Resolve an opaque activation against the latest model. Rejects unknown
    /// ids and wrong argument shapes; per-slot existence stays in
    /// [`command_enabled`]/slot-label gating, like settings range checks.
    pub fn resolve(&self, activation: CommandRef) -> Option<MenuCommand> {
        let key = self.key(activation.id)?;
        let template = key.template();
        match (template, activation.argument) {
            (command, None) if !key.is_indexed() => Some(command),
            (MenuCommand::Session(command), Some(index)) if key.is_indexed() => {
                Some(MenuCommand::Session(match command {
                    SessionCommand::SelectActiveSlot(_) => SessionCommand::SelectActiveSlot(index),
                    SessionCommand::SaveSlot(_) => SessionCommand::SaveSlot(index),
                    SessionCommand::LoadSlot(_) => SessionCommand::LoadSlot(index),
                    SessionCommand::DeleteSlot(_) => SessionCommand::DeleteSlot(index),
                    _ => return None,
                }))
            }
            _ => None,
        }
    }

    /// Reverse lookup for tests/examples: the opaque activation for a unit
    /// domain command, or the template (index 0) id for an indexed one.
    pub fn activation(&self, command: MenuCommand) -> Option<CommandRef> {
        let (key, argument) = command.binding();
        let (id, _) = self.0.iter().find(|(_, entry)| *entry == key)?;
        Some(CommandRef { id: *id, argument })
    }
}

/// Build the generic model plus its resolution table. Callers keep the
/// `IdRegistry` across frames so opaque ids stay stable while the model
/// (labels/enabled) refreshes, mirroring settings stable-id minting.
pub fn build_command_model(
    ids: &mut IdRegistry<MenuCommandId>,
    sig: &MenuSig,
) -> (CommandModel, CommandTable) {
    let mut table = CommandTable::default();
    let definitions = DECLARATIONS
        .iter()
        .map(|&(key, _, template)| {
            let id = ids.id(key);
            table.push(id, key);
            let spec: Box<dyn CommandSpec> = if key.is_indexed() {
                Box::new(IndexedSpec { index: 0 })
            } else {
                Box::new(UnitSpec)
            };
            Command {
                id,
                enabled: if key.is_indexed() {
                    sig.loaded && !sig.settings_open && !sig.slots.is_empty()
                } else {
                    command_enabled(template, sig)
                },
                spec,
            }
        })
        .collect();
    (CommandModel { definitions }, table)
}

pub fn command_enabled(command: MenuCommand, sig: &MenuSig) -> bool {
    use SessionCommand as S;
    if command == MenuCommand::Native(NativeRole::Quit) {
        return true;
    }
    !sig.settings_open
        && match command {
            MenuCommand::Open
            | MenuCommand::Settings
            | MenuCommand::Debugger
            | MenuCommand::NotificationHistory
            | MenuCommand::Native(_) => true,
            MenuCommand::Close => sig.loaded,
            MenuCommand::Session(command) => match command {
                S::Pause => sig.loaded && !sig.paused,
                S::Resume => sig.loaded && sig.paused,
                S::LoadActiveSlot => sig.loaded && sig.active_slot.is_some(),
                S::SelectActiveSlot(id) | S::SaveSlot(id) | S::LoadSlot(id) | S::DeleteSlot(id) => {
                    sig.loaded && sig.slots.iter().any(|slot| slot.slot_id == id)
                }
                S::SelectNextSlot | S::SelectPreviousSlot => sig.loaded && !sig.slots.is_empty(),
                S::Reset | S::CreateSlot | S::SaveActiveSlotOrNew | S::TogglePause => sig.loaded,
            },
        }
}

impl MenuCommand {
    fn template(self) -> (Self, Option<u64>) {
        use SessionCommand as S;
        match self {
            Self::Session(S::SelectActiveSlot(id)) => {
                (Self::Session(S::SelectActiveSlot(0)), Some(id))
            }
            Self::Session(S::SaveSlot(id)) => (Self::Session(S::SaveSlot(0)), Some(id)),
            Self::Session(S::LoadSlot(id)) => (Self::Session(S::LoadSlot(0)), Some(id)),
            Self::Session(S::DeleteSlot(id)) => (Self::Session(S::DeleteSlot(0)), Some(id)),
            other => (other, None),
        }
    }

    pub fn binding(self) -> (MenuCommandId, Option<u64>) {
        let (template, argument) = self.template();
        let key = DECLARATIONS
            .iter()
            .find(|(_, _, command)| *command == template)
            .expect("command declaration")
            .0;
        (key, argument)
    }
}

#[cfg(test)]
mod tests;

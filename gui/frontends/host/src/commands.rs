//! Canonical native command vocabulary, bindings and enablement rules.
//!
//! Framework-owned boundary language (MAUI `ICommand` analogue): the session
//! command enum lives here so shell, view definitions, and native frontends
//! share one vocabulary without a dependency cycle.

/// Session verbs shared by shell execution, view definitions, and frontends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionCommand {
    Pause,
    Resume,
    TogglePause,
    Reset,
    CreateSlot,
    SaveActiveSlotOrNew,
    LoadActiveSlot,
    SelectActiveSlot(u64),
    SaveSlot(u64),
    LoadSlot(u64),
    DeleteSlot(u64),
    SelectNextSlot,
    SelectPreviousSlot,
}

/// Canonical native command declarations, bindings and enablement.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeRole {
    About,
    Quit,
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommandId {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandParameter {
    None,
    SlotId,
}

// One declaration table supplies names, order, parameter types and bindings.
// Slot commands use zero as a template, never as a default argument.
/// All declarations for view-side model builders.
pub const DECLARATIONS: &[(CommandId, &str, MenuCommand)] = {
    use MenuCommand::{Native, Session};
    use SessionCommand as S;
    &[
        (CommandId::Open, "open", MenuCommand::Open),
        (CommandId::Close, "close", MenuCommand::Close),
        (CommandId::Settings, "settings", MenuCommand::Settings),
        (CommandId::Debugger, "debugger", MenuCommand::Debugger),
        (
            CommandId::NotificationHistory,
            "history",
            MenuCommand::NotificationHistory,
        ),
        (CommandId::Quit, "quit", Native(NativeRole::Quit)),
        (CommandId::About, "about", Native(NativeRole::About)),
        (CommandId::Pause, "pause", Session(S::Pause)),
        (CommandId::Resume, "resume", Session(S::Resume)),
        (CommandId::Reset, "reset", Session(S::Reset)),
        (
            CommandId::CreateSlot,
            "state-create",
            Session(S::CreateSlot),
        ),
        (
            CommandId::SaveActiveSlotOrNew,
            "state-save-active",
            Session(S::SaveActiveSlotOrNew),
        ),
        (
            CommandId::LoadActiveSlot,
            "state-load-active",
            Session(S::LoadActiveSlot),
        ),
        (
            CommandId::SelectActiveSlot,
            "state-select-slot",
            Session(S::SelectActiveSlot(0)),
        ),
        (
            CommandId::SaveSlot,
            "state-save-slot",
            Session(S::SaveSlot(0)),
        ),
        (
            CommandId::LoadSlot,
            "state-load-slot",
            Session(S::LoadSlot(0)),
        ),
        (
            CommandId::DeleteSlot,
            "state-delete-slot",
            Session(S::DeleteSlot(0)),
        ),
        (
            CommandId::TogglePause,
            "toggle-pause",
            Session(S::TogglePause),
        ),
        (
            CommandId::SelectNextSlot,
            "state-select-next",
            Session(S::SelectNextSlot),
        ),
        (
            CommandId::SelectPreviousSlot,
            "state-select-previous",
            Session(S::SelectPreviousSlot),
        ),
    ]
};

impl CommandId {
    fn declaration(self) -> &'static (Self, &'static str, MenuCommand) {
        DECLARATIONS
            .iter()
            .find(|(id, _, _)| *id == self)
            .expect("command declaration")
    }

    pub fn name(self) -> &'static str {
        self.declaration().1
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandDescriptor {
    pub id: CommandId,
    pub parameter: CommandParameter,
    pub enabled: bool,
}

impl CommandDescriptor {
    /// Binding is structural, not an execution guard. Disabled commands still
    /// bind; the host applies settings policy when an event actually arrives.
    pub fn bind(&self, argument: Option<u64>) -> Option<MenuCommand> {
        let template = self.id.declaration().2;
        let parameter = template.parameter();
        if self.parameter != parameter {
            return None;
        }
        match (template, parameter, argument) {
            (command, CommandParameter::None, None) => Some(command),
            (MenuCommand::Session(command), CommandParameter::SlotId, Some(id)) => {
                Some(MenuCommand::Session(match command {
                    SessionCommand::SelectActiveSlot(_) => SessionCommand::SelectActiveSlot(id),
                    SessionCommand::SaveSlot(_) => SessionCommand::SaveSlot(id),
                    SessionCommand::LoadSlot(_) => SessionCommand::LoadSlot(id),
                    SessionCommand::DeleteSlot(_) => SessionCommand::DeleteSlot(id),
                    _ => return None,
                }))
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandModel {
    pub definitions: Vec<CommandDescriptor>,
}

impl CommandModel {
    pub fn get(&self, id: CommandId) -> Option<&CommandDescriptor> {
        self.definitions.iter().find(|command| command.id == id)
    }

    pub fn visit(&self, visitor: &mut dyn CommandVisitor) {
        for command in &self.definitions {
            visitor.command(command);
        }
    }
}

pub trait CommandVisitor {
    fn command(&mut self, command: &CommandDescriptor);
}

impl MenuCommand {
    pub fn parameter(self) -> CommandParameter {
        if self.template().1.is_some() {
            CommandParameter::SlotId
        } else {
            CommandParameter::None
        }
    }

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

    pub fn binding(self) -> (CommandId, Option<u64>) {
        let (template, argument) = self.template();
        let id = DECLARATIONS
            .iter()
            .find(|(_, _, command)| *command == template)
            .expect("command declaration")
            .0;
        (id, argument)
    }
}

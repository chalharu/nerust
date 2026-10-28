//! Command model builders over the menu signature (view side).
//! Vocabulary (`MenuCommand`, `SessionCommand`, descriptors, visitors)
//! lives in the framework (`wakui::commands`).

use wakui::commands::{
    CommandDescriptor, CommandModel, CommandParameter, DECLARATIONS, MenuCommand, NativeRole,
};

use crate::menu::MenuSig;

pub fn build_command_model(sig: &MenuSig) -> CommandModel {
    CommandModel {
        definitions: DECLARATIONS
            .iter()
            .map(|&(id, _, command)| CommandDescriptor {
                id,
                parameter: command.parameter(),
                enabled: if command.parameter() == CommandParameter::SlotId {
                    sig.loaded && !sig.settings_open && !sig.slots.is_empty()
                } else {
                    command_enabled(command, sig)
                },
            })
            .collect(),
    }
}

pub fn command_enabled(command: MenuCommand, sig: &MenuSig) -> bool {
    use wakui::commands::SessionCommand as S;
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

#[cfg(test)]
mod tests;

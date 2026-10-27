//! Object-safe event boundary. Return owned native effects, never callbacks.

use nerust_frontend_kernel::pause::PauseOwner;
use nerust_gui_shell::session::{SessionError, access::SessionControl};

use crate::{
    application::ApplicationHost,
    commands::{MenuCommand, NativeRole},
    focus::{AuxKind, FocusEvent},
    session::{SessionEffect, SessionEvent, SessionOutcome},
};

#[derive(Debug, Clone, Copy)]
pub enum EventCommand {
    Menu(MenuCommand),
    Session(SessionEvent),
    Focus(FocusEvent),
    LiveTick,
}

/// False means the transport is dead. Rc-based native transports are welcome;
/// neither Send nor Sync is imposed on the UI boundary.
pub trait EventSink {
    fn send(&self, event: EventCommand) -> bool;
}

pub trait EventHandler {
    fn handle_event(
        &mut self,
        event: EventCommand,
        settings_open: bool,
    ) -> Result<EventOutcome, SessionError>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct EventOutcome {
    pub session: SessionOutcome,
    pub effect: Option<CommandEffect>,
    /// False for an event suppressed by settings policy.
    pub handled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandEffect {
    OpenRom,
    OpenAux(AuxKind),
    About,
    Quit,
    ToggleFullscreen,
    LiveTick,
}

impl EventHandler for ApplicationHost {
    fn handle_event(
        &mut self,
        event: EventCommand,
        settings_open: bool,
    ) -> Result<EventOutcome, SessionError> {
        let guarded = settings_open || self.owns_pause(PauseOwner::Settings);
        let blocked = match event {
            EventCommand::Menu(command) => command != MenuCommand::Native(NativeRole::Quit),
            EventCommand::Session(
                SessionEvent::Command(_)
                | SessionEvent::Keyboard { .. }
                | SessionEvent::Shortcut(_),
            ) => true,
            _ => false,
        };
        if guarded && blocked {
            return Ok(EventOutcome::default());
        }

        let mut outcome = EventOutcome {
            handled: true,
            ..Default::default()
        };
        match event {
            EventCommand::Menu(command) => match command {
                MenuCommand::Open => outcome.effect = Some(CommandEffect::OpenRom),
                MenuCommand::Close => SessionControl::unload(self),
                MenuCommand::Settings => {
                    outcome.effect = Some(CommandEffect::OpenAux(AuxKind::Settings));
                }
                MenuCommand::Debugger => {
                    outcome.effect = Some(CommandEffect::OpenAux(AuxKind::Debug));
                }
                MenuCommand::NotificationHistory => {
                    outcome.effect = Some(CommandEffect::OpenAux(AuxKind::History));
                }
                MenuCommand::Native(NativeRole::About) => {
                    outcome.effect = Some(CommandEffect::About);
                }
                MenuCommand::Native(NativeRole::Quit) => {
                    outcome.effect = Some(CommandEffect::Quit);
                }
                MenuCommand::Session(command) => {
                    outcome.session = self.dispatch(SessionEvent::Command(command))?;
                }
            },
            EventCommand::Session(event) => outcome.session = self.dispatch(event)?,
            EventCommand::Focus(event) => {
                self.focus_event(event);
            }
            EventCommand::LiveTick => outcome.effect = Some(CommandEffect::LiveTick),
        }
        if let Some(effect) = outcome.session.effect {
            outcome.effect = Some(match effect {
                SessionEffect::NativeOpenDebugger => CommandEffect::OpenAux(AuxKind::Debug),
                SessionEffect::ToggleFullscreen => CommandEffect::ToggleFullscreen,
            });
        }
        Ok(outcome)
    }
}

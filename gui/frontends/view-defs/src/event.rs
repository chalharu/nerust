//! Object-safe event boundary. Return owned native effects, never callbacks.

pub use nerust_frontend_host::{
    commands::{MenuCommand, NativeRole},
    event::{CommandEffect, EventCommand, EventHandler, EventOutcome, EventSink},
    session::{SessionError, SessionEvent},
};
use nerust_gui_shell::session::access::SessionControl;
use wakui::pause::PauseOwner;

use crate::{application::ApplicationHost, focus::AuxKind, session::SessionEffect};

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

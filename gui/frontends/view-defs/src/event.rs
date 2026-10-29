//! Object-safe event boundary. Return owned native effects, never callbacks.

use nerust_gui_shell::session::access::SessionControl;
use wakui::{
    input::event::{CommandEffect, EventCommand, EventHandler, EventOutcome},
    input::session::{SessionError, SessionEvent},
};

use crate::application::ApplicationHost;
use crate::commands::{MenuCommand, NativeRole};
use crate::focus::AuxKind;
use crate::session::DriverEffect;

impl EventHandler<AuxKind> for ApplicationHost {
    fn handle_event(&mut self, event: EventCommand) -> Result<EventOutcome<AuxKind>, SessionError> {
        let guarded = self.modal_guard_active();
        let blocked = match event {
            // Stale ids resolve to nothing and stay blocked: only Quit passes.
            EventCommand::Command(activation) => {
                self.menu_commands().resolve(activation)
                    != Some(MenuCommand::Native(NativeRole::Quit))
            }
            EventCommand::Session(SessionEvent::Command(_) | SessionEvent::Keyboard { .. }) => true,
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
            EventCommand::Command(activation) => {
                let Some(command) = self.menu_commands().resolve(activation) else {
                    return Ok(EventOutcome::default());
                };
                match command {
                    MenuCommand::Open => outcome.effect = Some(CommandEffect::OpenFile),
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
                    MenuCommand::Session(_) => {
                        let driver =
                            self.dispatch_driver_event(SessionEvent::Command(activation))?;
                        outcome.session = driver.into();
                    }
                }
            }
            EventCommand::Session(event) => {
                let driver = self.dispatch_driver_event(event)?;
                outcome.session = driver.into();
                match driver.effect {
                    Some(DriverEffect::OpenDebugger) => {
                        outcome.effect = Some(CommandEffect::OpenAux(AuxKind::Debug))
                    }
                    Some(DriverEffect::ToggleFullscreen) => {
                        outcome.effect = Some(CommandEffect::ToggleFullscreen)
                    }
                    None => {}
                }
            }
            EventCommand::Focus(event) => {
                self.focus_event(event);
            }
            EventCommand::LiveTick => outcome.effect = Some(CommandEffect::LiveTick),
        }
        Ok(outcome)
    }
}

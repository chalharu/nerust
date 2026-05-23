use crate::shell_api::{ButtonDescriptor, ControllerDescriptor, GuiSession};
use nerust_gui_runtime::{ConsoleSessionFactory, SessionCore, console_api::Console};
use nerust_screen_buffer::ScreenBuffer;
use nerust_sound_openal::OpenAl;
use nerust_timer::CLOCK_RATE;

#[derive(Debug, Clone, Copy, Default)]
pub struct NesConsoleDescriptor;

impl NesConsoleDescriptor {
    pub fn build_console(self) -> Console {
        let speaker = OpenAl::new(48_000, CLOCK_RATE as i32, 128, 20);
        Console::new(speaker, ScreenBuffer::new_nes_gpu_default())
    }

    pub fn build_session(&self) -> GuiSession {
        let core = SessionCore::from_console(self.build_console());
        GuiSession::from_session_core(core)
    }

    /// Returns the controller descriptor for the NES.
    ///
    /// Button names use the canonical NES names: **A** and **B** (not
    /// "Primary"/"Secondary"), matching the physical labels and the key
    /// mappings in [`crate::NesInputAdapter`].
    pub fn controller_descriptor(&self) -> ControllerDescriptor {
        ControllerDescriptor {
            port_count: 2,
            buttons: vec![
                ButtonDescriptor {
                    name: "A",
                    description: "Face button A",
                },
                ButtonDescriptor {
                    name: "B",
                    description: "Face button B",
                },
                ButtonDescriptor {
                    name: "Select",
                    description: "Select button",
                },
                ButtonDescriptor {
                    name: "Start",
                    description: "Start button",
                },
                ButtonDescriptor {
                    name: "Up",
                    description: "D-pad Up",
                },
                ButtonDescriptor {
                    name: "Down",
                    description: "D-pad Down",
                },
                ButtonDescriptor {
                    name: "Left",
                    description: "D-pad Left",
                },
                ButtonDescriptor {
                    name: "Right",
                    description: "D-pad Right",
                },
            ],
        }
    }
}

impl ConsoleSessionFactory for NesConsoleDescriptor {
    fn build_session(&self) -> GuiSession {
        NesConsoleDescriptor::build_session(self)
    }
}

#[cfg(test)]
mod tests {
    use super::NesConsoleDescriptor;

    #[test]
    fn nes_descriptor_has_canonical_ab_button_names() {
        let descriptor = NesConsoleDescriptor.controller_descriptor();
        let names: Vec<&str> = descriptor.buttons.iter().map(|b| b.name).collect();

        assert!(names.contains(&"A"), "expected A button in NES descriptor");
        assert!(names.contains(&"B"), "expected B button in NES descriptor");
        assert!(
            !names.contains(&"Primary"),
            "Primary is not a NES button name"
        );
        assert!(
            !names.contains(&"Secondary"),
            "Secondary is not a NES button name"
        );
        assert_eq!(descriptor.port_count, 2);
        assert_eq!(descriptor.buttons.len(), 8);
    }
}

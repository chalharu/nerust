/// A host-facing descriptor of a single controller button.
///
/// Button names use the console's canonical naming (e.g. "A", "B" for NES)
/// rather than generic names, so that UI labels match what users see on the
/// physical hardware.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ButtonDescriptor {
    /// The canonical button name shown to users (e.g. `"A"`, `"Start"`).
    pub name: &'static str,
    /// Short human-readable description of the button's function.
    pub description: &'static str,
}

/// A host-facing descriptor of a console's controller configuration.
///
/// This type lives in the shared session layer so that any shell or UI layer
/// can inspect the controller layout without depending on NES-specific
/// implementation details. NES-specific values are provided by
/// `NesConsoleDescriptor::controller_descriptor` in the `nerust_gui_shell`
/// crate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControllerDescriptor {
    /// Number of supported controller ports.
    pub port_count: usize,
    /// Ordered list of buttons present on each controller.
    pub buttons: Vec<ButtonDescriptor>,
}

#[cfg(test)]
mod tests {
    use super::{ButtonDescriptor, ControllerDescriptor};

    #[test]
    fn button_descriptor_has_expected_fields() {
        let btn = ButtonDescriptor {
            name: "A",
            description: "Face button A",
        };

        assert_eq!(btn.name, "A");
        assert_eq!(btn.description, "Face button A");
    }

    #[test]
    fn controller_descriptor_holds_button_list() {
        let desc = ControllerDescriptor {
            port_count: 2,
            buttons: vec![
                ButtonDescriptor {
                    name: "A",
                    description: "A",
                },
                ButtonDescriptor {
                    name: "B",
                    description: "B",
                },
            ],
        };

        assert_eq!(desc.port_count, 2);
        assert_eq!(desc.buttons.len(), 2);
    }
}

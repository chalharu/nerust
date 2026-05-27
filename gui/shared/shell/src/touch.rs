use nerust_gui_session::commands::SessionCommand;
use nerust_input_nes::topology::{
    NES_ATTACHMENT_PLAYER_ONE, NES_CONTROL_A, NES_CONTROL_B, NES_CONTROL_DOWN, NES_CONTROL_LEFT,
    NES_CONTROL_RIGHT, NES_CONTROL_SELECT, NES_CONTROL_START, NES_CONTROL_UP,
};
use nerust_input_schema::DigitalInputEvent;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TouchPoint {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TouchRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl TouchRect {
    fn contains(self, point: TouchPoint) -> bool {
        point.x >= self.x
            && point.x <= self.x + self.width
            && point.y >= self.y
            && point.y <= self.y + self.height
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchFrontendAction {
    OpenLibrary,
    OpenSettings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchOverlayAction {
    Input(DigitalInputEvent),
    Session(SessionCommand),
    Frontend(TouchFrontendAction),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchTarget {
    Up,
    Down,
    Left,
    Right,
    A,
    B,
    Start,
    Select,
    Pause,
    Reset,
    Save,
    Load,
    Library,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TouchZone {
    pub target: TouchTarget,
    pub bounds: TouchRect,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PortraitTouchOverlay {
    zones: Vec<TouchZone>,
}

impl PortraitTouchOverlay {
    pub fn new(width: f32, height: f32) -> Self {
        let control_top = height * 0.52;
        let control_height = height - control_top;
        let dpad_left = width * 0.06;
        let dpad_size = width * 0.30;
        let action_size = width * 0.16;
        let action_right = width * 0.76;
        let face_top = control_top + control_height * 0.10;
        let center_top = control_top + control_height * 0.38;

        let zones = vec![
            TouchZone {
                target: TouchTarget::Up,
                bounds: TouchRect {
                    x: dpad_left + dpad_size * 0.25,
                    y: control_top + control_height * 0.05,
                    width: dpad_size * 0.50,
                    height: dpad_size * 0.28,
                },
            },
            TouchZone {
                target: TouchTarget::Down,
                bounds: TouchRect {
                    x: dpad_left + dpad_size * 0.25,
                    y: control_top + control_height * 0.47,
                    width: dpad_size * 0.50,
                    height: dpad_size * 0.28,
                },
            },
            TouchZone {
                target: TouchTarget::Left,
                bounds: TouchRect {
                    x: dpad_left,
                    y: control_top + control_height * 0.26,
                    width: dpad_size * 0.28,
                    height: dpad_size * 0.36,
                },
            },
            TouchZone {
                target: TouchTarget::Right,
                bounds: TouchRect {
                    x: dpad_left + dpad_size * 0.47,
                    y: control_top + control_height * 0.26,
                    width: dpad_size * 0.28,
                    height: dpad_size * 0.36,
                },
            },
            TouchZone {
                target: TouchTarget::B,
                bounds: TouchRect {
                    x: action_right - action_size * 1.1,
                    y: face_top + action_size * 0.55,
                    width: action_size,
                    height: action_size,
                },
            },
            TouchZone {
                target: TouchTarget::A,
                bounds: TouchRect {
                    x: action_right,
                    y: face_top,
                    width: action_size,
                    height: action_size,
                },
            },
            TouchZone {
                target: TouchTarget::Select,
                bounds: TouchRect {
                    x: width * 0.36,
                    y: center_top,
                    width: width * 0.12,
                    height: height * 0.05,
                },
            },
            TouchZone {
                target: TouchTarget::Start,
                bounds: TouchRect {
                    x: width * 0.52,
                    y: center_top,
                    width: width * 0.12,
                    height: height * 0.05,
                },
            },
            TouchZone {
                target: TouchTarget::Pause,
                bounds: TouchRect {
                    x: width * 0.72,
                    y: control_top + control_height * 0.72,
                    width: width * 0.10,
                    height: height * 0.05,
                },
            },
            TouchZone {
                target: TouchTarget::Reset,
                bounds: TouchRect {
                    x: width * 0.84,
                    y: control_top + control_height * 0.72,
                    width: width * 0.10,
                    height: height * 0.05,
                },
            },
            TouchZone {
                target: TouchTarget::Save,
                bounds: TouchRect {
                    x: width * 0.72,
                    y: control_top + control_height * 0.82,
                    width: width * 0.10,
                    height: height * 0.05,
                },
            },
            TouchZone {
                target: TouchTarget::Load,
                bounds: TouchRect {
                    x: width * 0.84,
                    y: control_top + control_height * 0.82,
                    width: width * 0.10,
                    height: height * 0.05,
                },
            },
            TouchZone {
                target: TouchTarget::Library,
                bounds: TouchRect {
                    x: width * 0.06,
                    y: control_top + control_height * 0.82,
                    width: width * 0.14,
                    height: height * 0.05,
                },
            },
            TouchZone {
                target: TouchTarget::Settings,
                bounds: TouchRect {
                    x: width * 0.22,
                    y: control_top + control_height * 0.82,
                    width: width * 0.14,
                    height: height * 0.05,
                },
            },
        ];

        Self { zones }
    }

    pub fn zones(&self) -> &[TouchZone] {
        &self.zones
    }

    pub fn hit_test(&self, point: TouchPoint) -> Option<TouchTarget> {
        self.zones
            .iter()
            .find(|zone| zone.bounds.contains(point))
            .map(|zone| zone.target)
    }
}

pub fn actions_for_target(target: TouchTarget, pressed: bool) -> Vec<TouchOverlayAction> {
    let input = |control| {
        if pressed {
            TouchOverlayAction::Input(DigitalInputEvent::pressed(
                NES_ATTACHMENT_PLAYER_ONE,
                control,
            ))
        } else {
            TouchOverlayAction::Input(DigitalInputEvent::released(
                NES_ATTACHMENT_PLAYER_ONE,
                control,
            ))
        }
    };

    match target {
        TouchTarget::Up => vec![input(NES_CONTROL_UP)],
        TouchTarget::Down => vec![input(NES_CONTROL_DOWN)],
        TouchTarget::Left => vec![input(NES_CONTROL_LEFT)],
        TouchTarget::Right => vec![input(NES_CONTROL_RIGHT)],
        TouchTarget::A => vec![input(NES_CONTROL_A)],
        TouchTarget::B => vec![input(NES_CONTROL_B)],
        TouchTarget::Start => vec![input(NES_CONTROL_START)],
        TouchTarget::Select => vec![input(NES_CONTROL_SELECT)],
        TouchTarget::Pause if pressed => {
            vec![TouchOverlayAction::Session(SessionCommand::TogglePause)]
        }
        TouchTarget::Reset if pressed => vec![TouchOverlayAction::Session(SessionCommand::Reset)],
        TouchTarget::Save if pressed => {
            vec![TouchOverlayAction::Session(
                SessionCommand::SaveActiveSlotOrNew,
            )]
        }
        TouchTarget::Load if pressed => {
            vec![TouchOverlayAction::Session(SessionCommand::LoadActiveSlot)]
        }
        TouchTarget::Library if pressed => {
            vec![TouchOverlayAction::Frontend(
                TouchFrontendAction::OpenLibrary,
            )]
        }
        TouchTarget::Settings if pressed => {
            vec![TouchOverlayAction::Frontend(
                TouchFrontendAction::OpenSettings,
            )]
        }
        TouchTarget::Pause
        | TouchTarget::Reset
        | TouchTarget::Save
        | TouchTarget::Load
        | TouchTarget::Library
        | TouchTarget::Settings => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        PortraitTouchOverlay, TouchFrontendAction, TouchOverlayAction, TouchPoint, TouchTarget,
        actions_for_target,
    };
    use nerust_gui_session::commands::SessionCommand;
    use nerust_input_nes::topology::{
        NES_ATTACHMENT_PLAYER_ONE, NES_CONTROL_A, NES_CONTROL_LEFT, NES_CONTROL_UP,
    };
    use nerust_input_schema::DigitalInputEvent;

    #[test]
    fn portrait_layout_maps_points_to_expected_targets() {
        let overlay = PortraitTouchOverlay::new(1080.0, 1920.0);

        assert_eq!(
            overlay.hit_test(TouchPoint {
                x: 200.0,
                y: 1100.0
            }),
            Some(TouchTarget::Up)
        );
        assert_eq!(
            overlay.hit_test(TouchPoint {
                x: 900.0,
                y: 1180.0
            }),
            Some(TouchTarget::A)
        );
        // Library is at x=[64.8, 216], y≈[1754, 1850]
        assert_eq!(
            overlay.hit_test(TouchPoint {
                x: 120.0,
                y: 1760.0
            }),
            Some(TouchTarget::Library)
        );
        // Settings is at x=[237.6, 388.8], same y band
        assert_eq!(
            overlay.hit_test(TouchPoint {
                x: 300.0,
                y: 1760.0
            }),
            Some(TouchTarget::Settings)
        );
        assert_eq!(overlay.hit_test(TouchPoint { x: 50.0, y: 200.0 }), None);
    }

    #[test]
    fn button_targets_translate_to_pressed_and_released_input() {
        assert_eq!(
            actions_for_target(TouchTarget::A, true),
            vec![TouchOverlayAction::Input(DigitalInputEvent::pressed(
                NES_ATTACHMENT_PLAYER_ONE,
                NES_CONTROL_A
            ))]
        );
        assert_eq!(
            actions_for_target(TouchTarget::Left, false),
            vec![TouchOverlayAction::Input(DigitalInputEvent::released(
                NES_ATTACHMENT_PLAYER_ONE,
                NES_CONTROL_LEFT
            ))]
        );
        assert_eq!(
            actions_for_target(TouchTarget::Up, true),
            vec![TouchOverlayAction::Input(DigitalInputEvent::pressed(
                NES_ATTACHMENT_PLAYER_ONE,
                NES_CONTROL_UP
            ))]
        );
    }

    #[test]
    fn non_hold_targets_emit_actions_only_on_press() {
        assert_eq!(
            actions_for_target(TouchTarget::Pause, true),
            vec![TouchOverlayAction::Session(SessionCommand::TogglePause)]
        );
        assert!(actions_for_target(TouchTarget::Pause, false).is_empty());
        assert_eq!(
            actions_for_target(TouchTarget::Library, true),
            vec![TouchOverlayAction::Frontend(
                TouchFrontendAction::OpenLibrary
            )]
        );
    }

    #[test]
    fn settings_target_emits_open_settings_on_press_only() {
        assert_eq!(
            actions_for_target(TouchTarget::Settings, true),
            vec![TouchOverlayAction::Frontend(
                TouchFrontendAction::OpenSettings
            )]
        );
        assert!(actions_for_target(TouchTarget::Settings, false).is_empty());
    }

    #[test]
    fn portrait_overlay_includes_settings_zone() {
        let overlay = PortraitTouchOverlay::new(1080.0, 1920.0);
        let zones = overlay.zones();
        assert!(
            zones.iter().any(|z| z.target == TouchTarget::Settings),
            "expected a Settings zone in the portrait overlay"
        );
        assert!(
            zones.iter().any(|z| z.target == TouchTarget::Library),
            "expected a Library zone in the portrait overlay"
        );
    }
}

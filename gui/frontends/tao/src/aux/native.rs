//! Single tao boundary: native ids plus one event adapter.
//! Domain layers speak domain types only; tao never crosses outward.

use tao::{event::WindowEvent, window::WindowId};

/// Opaque native window identity for registry routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct NativeWindowId(WindowId);

impl NativeWindowId {
    pub(crate) fn new(id: WindowId) -> Self {
        Self(id)
    }

    /// Test-only dummy id: distinctness across dummies is not guaranteed.
    #[cfg(test)]
    pub(crate) fn dummy() -> Self {
        Self(unsafe { WindowId::dummy() })
    }
}

/// Registry-assigned aux entry handle. Routing keys on this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct AuxId(u64);

impl AuxId {
    pub(crate) fn new(raw: u64) -> Self {
        Self(raw)
    }
}

/// Domain-level aux event: plain data, no tao import outward.
/// Unmapped keys and buttons are dropped by the adapter.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum AuxNativeEvent {
    Focused(bool),
    CloseRequested,
    Destroyed,
    Moved(i32, i32),
    Resized(u32, u32),
    ScaleChanged(f32),
    ModifiersChanged(iced::keyboard::Modifiers),
    CursorMoved(f64, f64),
    CursorLeft,
    MousePressed(super::tao_map::AuxButton),
    MouseReleased(super::tao_map::AuxButton),
    WheelLines(f32, f32),
    WheelPixels(f32, f32),
    KeyPressed {
        key: iced::keyboard::Key,
        physical: iced::keyboard::key::Physical,
        text: Option<String>,
        repeat: bool,
    },
    KeyReleased {
        key: iced::keyboard::Key,
        physical: iced::keyboard::key::Physical,
    },
    Other,
}

/// Map one native event to the domain event. Cursor and wheel
/// pixels stay physical: the shell divides by its live scale.
pub(crate) fn adapt_event(event: &WindowEvent) -> AuxNativeEvent {
    match event {
        WindowEvent::Focused(focused) => AuxNativeEvent::Focused(*focused),
        WindowEvent::CloseRequested => AuxNativeEvent::CloseRequested,
        WindowEvent::Destroyed => AuxNativeEvent::Destroyed,
        WindowEvent::Moved(position) => AuxNativeEvent::Moved(position.x, position.y),
        WindowEvent::Resized(size) => AuxNativeEvent::Resized(size.width, size.height),
        WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
            AuxNativeEvent::ScaleChanged(*scale_factor as f32)
        }
        WindowEvent::ModifiersChanged(state) => {
            AuxNativeEvent::ModifiersChanged(super::tao_map::tao_modifiers_to_iced(*state))
        }
        WindowEvent::CursorMoved { position, .. } => {
            AuxNativeEvent::CursorMoved(position.x, position.y)
        }
        WindowEvent::CursorLeft { .. } => AuxNativeEvent::CursorLeft,
        WindowEvent::MouseInput { button, state, .. } => {
            let pressed = match state {
                tao::event::ElementState::Pressed => true,
                tao::event::ElementState::Released => false,
                _ => return AuxNativeEvent::Other,
            };
            match super::tao_map::tao_button_to_aux(*button) {
                Some(button) if pressed => AuxNativeEvent::MousePressed(button),
                Some(button) => AuxNativeEvent::MouseReleased(button),
                None => AuxNativeEvent::Other,
            }
        }
        WindowEvent::MouseWheel { delta, .. } => match delta {
            tao::event::MouseScrollDelta::LineDelta(x, y) => AuxNativeEvent::WheelLines(*x, *y),
            tao::event::MouseScrollDelta::PixelDelta(position) => {
                AuxNativeEvent::WheelPixels(position.x as f32, position.y as f32)
            }
            _ => AuxNativeEvent::Other,
        },
        WindowEvent::KeyboardInput { event: ke, .. } => {
            let Some(physical) = super::tao_map::tao_keycode_to_iced_code(ke.physical_key) else {
                return AuxNativeEvent::Other;
            };
            let physical = iced::keyboard::key::Physical::Code(physical);
            let key = super::tao_map::tao_key_to_iced_key(&ke.logical_key);
            match ke.state {
                tao::event::ElementState::Pressed => AuxNativeEvent::KeyPressed {
                    key: key.clone(),
                    physical,
                    text: ke.text.as_ref().map(|s| s.to_string()),
                    repeat: ke.repeat,
                },
                tao::event::ElementState::Released => AuxNativeEvent::KeyReleased { key, physical },
                _ => AuxNativeEvent::Other,
            }
        }
        _ => AuxNativeEvent::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aux_ids_compare_by_value() {
        assert_eq!(AuxId::new(1), AuxId::new(1));
        assert_ne!(AuxId::new(1), AuxId::new(2));
    }

    #[test]
    fn adapter_maps_focus_and_lifecycle() {
        assert_eq!(
            adapt_event(&WindowEvent::Focused(true)),
            AuxNativeEvent::Focused(true)
        );
        assert_eq!(
            adapt_event(&WindowEvent::CloseRequested),
            AuxNativeEvent::CloseRequested
        );
        assert_eq!(
            adapt_event(&WindowEvent::Destroyed),
            AuxNativeEvent::Destroyed
        );
        assert_eq!(
            adapt_event(&WindowEvent::Resized(tao::dpi::PhysicalSize::new(800, 600))),
            AuxNativeEvent::Resized(800, 600)
        );
    }

    #[test]
    fn adapter_keeps_cursor_physical_for_shell_scale() {
        let event = WindowEvent::CursorMoved {
            device_id: unsafe { tao::event::DeviceId::dummy() },
            position: tao::dpi::PhysicalPosition::new(200.0, 100.0),
            modifiers: tao::keyboard::ModifiersState::default(),
        };
        // Physical pixels cross; the shell divides by its live scale.
        assert_eq!(
            adapt_event(&event),
            AuxNativeEvent::CursorMoved(200.0, 100.0)
        );
    }

    #[test]
    fn adapter_drops_unmapped_buttons() {
        let mouse = WindowEvent::MouseInput {
            device_id: unsafe { tao::event::DeviceId::dummy() },
            state: tao::event::ElementState::Pressed,
            button: tao::event::MouseButton::Other(9),
            modifiers: tao::keyboard::ModifiersState::default(),
        };
        assert_eq!(adapt_event(&mouse), AuxNativeEvent::Other);
        let left = WindowEvent::MouseInput {
            device_id: unsafe { tao::event::DeviceId::dummy() },
            state: tao::event::ElementState::Released,
            button: tao::event::MouseButton::Left,
            modifiers: tao::keyboard::ModifiersState::default(),
        };
        assert_eq!(
            adapt_event(&left),
            AuxNativeEvent::MouseReleased(crate::aux::tao_map::AuxButton::Left)
        );
    }

    #[test]
    fn adapter_maps_geometry_and_modifier_events() {
        assert_eq!(
            adapt_event(&WindowEvent::Moved(tao::dpi::PhysicalPosition::new(7, 9))),
            AuxNativeEvent::Moved(7, 9)
        );
        assert_eq!(
            adapt_event(&WindowEvent::ScaleFactorChanged {
                scale_factor: 2.0,
                new_inner_size: &mut tao::dpi::PhysicalSize::new(800, 600),
            }),
            AuxNativeEvent::ScaleChanged(2.0)
        );
        // Tao modifiers cross as a set; empty stays empty.
        assert_eq!(
            adapt_event(&WindowEvent::ModifiersChanged(
                tao::keyboard::ModifiersState::SHIFT
            )),
            AuxNativeEvent::ModifiersChanged(crate::aux::tao_map::tao_modifiers_to_iced(
                tao::keyboard::ModifiersState::SHIFT
            ))
        );
        assert_eq!(
            adapt_event(&WindowEvent::ModifiersChanged(
                tao::keyboard::ModifiersState::empty()
            )),
            AuxNativeEvent::ModifiersChanged(iced::keyboard::Modifiers::empty())
        );
        assert_eq!(
            adapt_event(&WindowEvent::CursorLeft {
                device_id: unsafe { tao::event::DeviceId::dummy() },
            }),
            AuxNativeEvent::CursorLeft
        );
    }

    #[test]
    fn adapter_maps_each_mapped_button_press() {
        use crate::aux::tao_map::AuxButton;
        for (native, domain) in [
            (tao::event::MouseButton::Left, AuxButton::Left),
            (tao::event::MouseButton::Right, AuxButton::Right),
            (tao::event::MouseButton::Middle, AuxButton::Middle),
        ] {
            let pressed = WindowEvent::MouseInput {
                device_id: unsafe { tao::event::DeviceId::dummy() },
                state: tao::event::ElementState::Pressed,
                button: native,
                modifiers: tao::keyboard::ModifiersState::default(),
            };
            assert_eq!(adapt_event(&pressed), AuxNativeEvent::MousePressed(domain));
            let released = WindowEvent::MouseInput {
                device_id: unsafe { tao::event::DeviceId::dummy() },
                state: tao::event::ElementState::Released,
                button: native,
                modifiers: tao::keyboard::ModifiersState::default(),
            };
            assert_eq!(
                adapt_event(&released),
                AuxNativeEvent::MouseReleased(domain)
            );
        }
    }

    #[test]
    fn adapter_maps_both_wheel_deltas() {
        let device = unsafe { tao::event::DeviceId::dummy() };
        let lines = WindowEvent::MouseWheel {
            device_id: device,
            delta: tao::event::MouseScrollDelta::LineDelta(0.0, 3.0),
            phase: tao::event::TouchPhase::Moved,
            modifiers: tao::keyboard::ModifiersState::default(),
        };
        assert_eq!(adapt_event(&lines), AuxNativeEvent::WheelLines(0.0, 3.0));
        let pixels = WindowEvent::MouseWheel {
            device_id: device,
            delta: tao::event::MouseScrollDelta::PixelDelta(tao::dpi::PhysicalPosition::new(
                0.0, 24.0,
            )),
            phase: tao::event::TouchPhase::Moved,
            modifiers: tao::keyboard::ModifiersState::default(),
        };
        assert_eq!(adapt_event(&pixels), AuxNativeEvent::WheelPixels(0.0, 24.0));
    }

    #[test]
    fn adapter_passes_unmapped_window_events_through() {
        // Non-input lifecycle events never reach the programs.
        assert_eq!(
            adapt_event(&WindowEvent::ThemeChanged(tao::window::Theme::Dark)),
            AuxNativeEvent::Other
        );
        assert_eq!(
            adapt_event(&WindowEvent::DecorationsClick),
            AuxNativeEvent::Other
        );
    }
}

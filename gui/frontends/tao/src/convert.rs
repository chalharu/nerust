use std::sync::atomic::AtomicBool;

use iced::{Event, Font, Point, keyboard, mouse};
use iced_winit::core::SmolStr;
use nerust_keyboard::Key;
use tao::keyboard::{KeyCode, ModifiersState as TaoModifiers};

/// Convert Tao modifiers to iced modifiers.
pub(crate) fn tao_modifiers_to_iced(m: TaoModifiers) -> keyboard::Modifiers {
    let mut out = keyboard::Modifiers::empty();
    out.set(keyboard::Modifiers::SHIFT, m.contains(TaoModifiers::SHIFT));
    out.set(keyboard::Modifiers::CTRL, m.contains(TaoModifiers::CONTROL));
    out.set(keyboard::Modifiers::ALT, m.contains(TaoModifiers::ALT));
    out.set(keyboard::Modifiers::LOGO, m.contains(TaoModifiers::SUPER));
    out
}

/// Convert a Tao physical key code to the shared [`Key`]: `None` for unhandled codes.
pub(crate) fn keycode_to_key(code: tao::keyboard::KeyCode) -> Option<Key> {
    Some(match code {
        KeyCode::KeyA => Key::KeyA,
        KeyCode::KeyB => Key::KeyB,
        KeyCode::KeyC => Key::KeyC,
        KeyCode::KeyD => Key::KeyD,
        KeyCode::KeyE => Key::KeyE,
        KeyCode::KeyF => Key::KeyF,
        KeyCode::KeyG => Key::KeyG,
        KeyCode::KeyH => Key::KeyH,
        KeyCode::KeyI => Key::KeyI,
        KeyCode::KeyJ => Key::KeyJ,
        KeyCode::KeyK => Key::KeyK,
        KeyCode::KeyL => Key::KeyL,
        KeyCode::KeyM => Key::KeyM,
        KeyCode::KeyN => Key::KeyN,
        KeyCode::KeyO => Key::KeyO,
        KeyCode::KeyP => Key::KeyP,
        KeyCode::KeyQ => Key::KeyQ,
        KeyCode::KeyR => Key::KeyR,
        KeyCode::KeyS => Key::KeyS,
        KeyCode::KeyT => Key::KeyT,
        KeyCode::KeyU => Key::KeyU,
        KeyCode::KeyV => Key::KeyV,
        KeyCode::KeyW => Key::KeyW,
        KeyCode::KeyX => Key::KeyX,
        KeyCode::KeyY => Key::KeyY,
        KeyCode::KeyZ => Key::KeyZ,
        KeyCode::Digit0 => Key::Digit0,
        KeyCode::Digit1 => Key::Digit1,
        KeyCode::Digit2 => Key::Digit2,
        KeyCode::Digit3 => Key::Digit3,
        KeyCode::Digit4 => Key::Digit4,
        KeyCode::Digit5 => Key::Digit5,
        KeyCode::Digit6 => Key::Digit6,
        KeyCode::Digit7 => Key::Digit7,
        KeyCode::Digit8 => Key::Digit8,
        KeyCode::Digit9 => Key::Digit9,
        KeyCode::ArrowUp => Key::ArrowUp,
        KeyCode::ArrowDown => Key::ArrowDown,
        KeyCode::ArrowLeft => Key::ArrowLeft,
        KeyCode::ArrowRight => Key::ArrowRight,
        KeyCode::Enter => Key::Enter,
        KeyCode::Escape => Key::Escape,
        KeyCode::Space => Key::Space,
        KeyCode::Tab => Key::Tab,
        KeyCode::F1 => Key::F1,
        KeyCode::F2 => Key::F2,
        KeyCode::F3 => Key::F3,
        KeyCode::F4 => Key::F4,
        KeyCode::F5 => Key::F5,
        KeyCode::F6 => Key::F6,
        KeyCode::F7 => Key::F7,
        KeyCode::F8 => Key::F8,
        KeyCode::F9 => Key::F9,
        KeyCode::F10 => Key::F10,
        KeyCode::F11 => Key::F11,
        KeyCode::F12 => Key::F12,
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Delete => Key::Delete,
        KeyCode::Home => Key::Home,
        KeyCode::End => Key::End,
        KeyCode::ShiftLeft => Key::ShiftLeft,
        KeyCode::ShiftRight => Key::ShiftRight,
        KeyCode::ControlLeft => Key::ControlLeft,
        KeyCode::ControlRight => Key::ControlRight,
        KeyCode::AltLeft => Key::AltLeft,
        KeyCode::AltRight => Key::AltRight,
        KeyCode::Backquote => Key::Backquote,
        KeyCode::Comma => Key::Comma,
        KeyCode::Period => Key::Period,
        KeyCode::Slash => Key::Slash,
        KeyCode::Semicolon => Key::Semicolon,
        KeyCode::Quote => Key::Quote,
        KeyCode::BracketLeft => Key::BracketLeft,
        KeyCode::BracketRight => Key::BracketRight,
        KeyCode::Backslash => Key::Backslash,
        KeyCode::Minus => Key::Minus,
        KeyCode::Equal => Key::Equal,
        KeyCode::IntlRo => Key::IntlRo,
        KeyCode::IntlYen => Key::IntlYen,
        KeyCode::Hiragana => Key::Hiragana,
        KeyCode::Katakana => Key::Katakana,
        KeyCode::KanaMode => Key::KanaMode,
        KeyCode::Numpad0 => Key::Numpad0,
        KeyCode::Numpad1 => Key::Numpad1,
        KeyCode::Numpad2 => Key::Numpad2,
        KeyCode::Numpad3 => Key::Numpad3,
        KeyCode::Numpad4 => Key::Numpad4,
        KeyCode::Numpad5 => Key::Numpad5,
        KeyCode::Numpad6 => Key::Numpad6,
        KeyCode::Numpad7 => Key::Numpad7,
        KeyCode::Numpad8 => Key::Numpad8,
        KeyCode::Numpad9 => Key::Numpad9,
        KeyCode::NumpadDecimal => Key::NumpadDecimal,
        KeyCode::NumpadAdd => Key::NumpadAdd,
        KeyCode::NumpadSubtract => Key::NumpadSubtract,
        KeyCode::NumpadMultiply => Key::NumpadMultiply,
        KeyCode::NumpadDivide => Key::NumpadDivide,
        KeyCode::NumpadEnter => Key::NumpadEnter,
        KeyCode::Insert => Key::Insert,
        KeyCode::IntlBackslash => Key::IntlBackslash,
        KeyCode::PageUp => Key::PageUp,
        KeyCode::PageDown => Key::PageDown,
        KeyCode::CapsLock => Key::CapsLock,
        KeyCode::ScrollLock => Key::ScrollLock,
        KeyCode::Pause => Key::Pause,
        KeyCode::PrintScreen => Key::PrintScreen,
        KeyCode::Convert => Key::Convert,
        KeyCode::NonConvert => Key::NonConvert,
        KeyCode::F13 => Key::F13,
        KeyCode::F14 => Key::F14,
        KeyCode::F15 => Key::F15,
        KeyCode::F16 => Key::F16,
        KeyCode::F17 => Key::F17,
        KeyCode::F18 => Key::F18,
        KeyCode::F19 => Key::F19,
        KeyCode::F20 => Key::F20,
        KeyCode::F21 => Key::F21,
        KeyCode::F22 => Key::F22,
        KeyCode::F23 => Key::F23,
        KeyCode::F24 => Key::F24,
        KeyCode::F25 => Key::F25,
        KeyCode::F26 => Key::F26,
        KeyCode::F27 => Key::F27,
        KeyCode::F28 => Key::F28,
        KeyCode::F29 => Key::F29,
        KeyCode::F30 => Key::F30,
        KeyCode::F31 => Key::F31,
        KeyCode::F32 => Key::F32,
        KeyCode::F33 => Key::F33,
        KeyCode::F34 => Key::F34,
        KeyCode::F35 => Key::F35,
        KeyCode::Plus => Key::Plus,
        KeyCode::NumpadEqual => Key::NumpadEqual,
        KeyCode::NumpadComma => Key::NumpadComma,
        KeyCode::NumpadBackspace => Key::NumpadBackspace,
        KeyCode::NumpadClear => Key::NumpadClear,
        KeyCode::NumpadClearEntry => Key::NumpadClearEntry,
        KeyCode::NumpadHash => Key::NumpadHash,
        KeyCode::NumpadMemoryAdd => Key::NumpadMemoryAdd,
        KeyCode::NumpadMemoryClear => Key::NumpadMemoryClear,
        KeyCode::NumpadMemoryRecall => Key::NumpadMemoryRecall,
        KeyCode::NumpadMemoryStore => Key::NumpadMemoryStore,
        KeyCode::NumpadMemorySubtract => Key::NumpadMemorySubtract,
        KeyCode::NumpadParenLeft => Key::NumpadParenLeft,
        KeyCode::NumpadParenRight => Key::NumpadParenRight,
        KeyCode::NumpadStar => Key::NumpadStar,
        KeyCode::SuperLeft => Key::SuperLeft,
        KeyCode::SuperRight => Key::SuperRight,
        KeyCode::AudioVolumeUp => Key::AudioVolumeUp,
        KeyCode::AudioVolumeDown => Key::AudioVolumeDown,
        KeyCode::AudioVolumeMute => Key::AudioVolumeMute,
        KeyCode::MediaPlayPause => Key::MediaPlayPause,
        KeyCode::MediaStop => Key::MediaStop,
        KeyCode::MediaTrackNext => Key::MediaTrackNext,
        KeyCode::MediaTrackPrevious => Key::MediaTrackPrevious,
        KeyCode::MediaSelect => Key::MediaSelect,
        KeyCode::BrowserBack => Key::BrowserBack,
        KeyCode::BrowserFavorites => Key::BrowserFavorites,
        KeyCode::BrowserForward => Key::BrowserForward,
        KeyCode::BrowserHome => Key::BrowserHome,
        KeyCode::BrowserRefresh => Key::BrowserRefresh,
        KeyCode::BrowserSearch => Key::BrowserSearch,
        KeyCode::BrowserStop => Key::BrowserStop,
        KeyCode::NumLock => Key::NumLock,
        KeyCode::ContextMenu => Key::ContextMenu,
        KeyCode::Power => Key::Power,
        KeyCode::Sleep => Key::Sleep,
        KeyCode::WakeUp => Key::WakeUp,
        KeyCode::Eject => Key::Eject,
        KeyCode::Copy => Key::Copy,
        KeyCode::Cut => Key::Cut,
        KeyCode::Paste => Key::Paste,
        KeyCode::Undo => Key::Undo,
        KeyCode::Find => Key::Find,
        KeyCode::Help => Key::Help,
        KeyCode::Open => Key::Open,
        KeyCode::Select => Key::Select,
        KeyCode::Again => Key::Again,
        KeyCode::Props => Key::Props,
        KeyCode::Abort => Key::Abort,
        KeyCode::Resume => Key::Resume,
        KeyCode::Suspend => Key::Suspend,
        KeyCode::Fn => Key::Fn,
        KeyCode::FnLock => Key::FnLock,
        KeyCode::Hyper => Key::Hyper,
        KeyCode::Turbo => Key::Turbo,
        KeyCode::Lang1 => Key::Lang1,
        KeyCode::Lang2 => Key::Lang2,
        KeyCode::Lang3 => Key::Lang3,
        KeyCode::Lang4 => Key::Lang4,
        KeyCode::Lang5 => Key::Lang5,
        KeyCode::LaunchApp1 => Key::LaunchApp1,
        KeyCode::LaunchApp2 => Key::LaunchApp2,
        KeyCode::LaunchMail => Key::LaunchMail,
        _ => return None,
    })
}

/// Convert Tao KeyCode to iced key::Code.
pub(crate) fn tao_keycode_to_iced_code(code: tao::keyboard::KeyCode) -> keyboard::key::Code {
    let key = keycode_to_key(code).unwrap_or(Key::Backquote);
    key.into()
}

/// Convert Tao Key to iced Key. Named editing keys map explicitly;
/// anything else stays unidentified (physical codes cover shortcuts).
pub(crate) fn tao_key_to_iced_key(key: &tao::keyboard::Key) -> keyboard::Key {
    use iced::keyboard::key::Named;
    match key {
        tao::keyboard::Key::Character(s) => keyboard::Key::Character(SmolStr::new(s)),
        tao::keyboard::Key::Backspace => keyboard::Key::Named(Named::Backspace),
        tao::keyboard::Key::Delete => keyboard::Key::Named(Named::Delete),
        tao::keyboard::Key::Enter => keyboard::Key::Named(Named::Enter),
        tao::keyboard::Key::Escape => keyboard::Key::Named(Named::Escape),
        tao::keyboard::Key::Tab => keyboard::Key::Named(Named::Tab),
        _ => keyboard::Key::Unidentified,
    }
}

/// Default font for auxiliary windows: CJK resolves to a complete sans face.
/// Language-specific needs live in `nerust_settings_core::i18n::font_family`.
pub(crate) fn default_font() -> Font {
    #[cfg(target_os = "windows")]
    {
        Font::with_name("Yu Gothic UI")
    }
    #[cfg(target_os = "macos")]
    {
        Font::with_name("Hiragino Sans")
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        Font::DEFAULT
    }
}

/// Convert a Tao window event to an iced event, tracking cursor, modifiers, close.
pub(crate) fn convert_tao_window_event(
    event: tao::event::WindowEvent,
    cursor: &mut mouse::Cursor,
    scale_factor: f32,
    modifiers: &mut keyboard::Modifiers,
    should_close: &AtomicBool,
) -> Option<iced::Event> {
    use tao::event::WindowEvent;
    match event {
        WindowEvent::CursorMoved { position, .. } => {
            let logical = position.to_logical::<f64>(scale_factor as f64);
            let point = Point::new(logical.x as f32, logical.y as f32);
            *cursor = mouse::Cursor::Available(point);
            Some(Event::Mouse(mouse::Event::CursorMoved { position: point }))
        }
        WindowEvent::CursorLeft { .. } => {
            *cursor = mouse::Cursor::Unavailable;
            None
        }
        WindowEvent::KeyboardInput { event: ke, .. } => {
            let iced_key = tao_key_to_iced_key(&ke.logical_key);
            let physical_key =
                keyboard::key::Physical::Code(tao_keycode_to_iced_code(ke.physical_key));
            match ke.state {
                tao::event::ElementState::Pressed => {
                    Some(Event::Keyboard(keyboard::Event::KeyPressed {
                        key: iced_key.clone(),
                        modified_key: iced_key.clone(),
                        physical_key,
                        modifiers: *modifiers,
                        location: keyboard::Location::Standard,
                        text: ke.text.map(SmolStr::new),
                        repeat: ke.repeat,
                    }))
                }
                tao::event::ElementState::Released => {
                    Some(Event::Keyboard(keyboard::Event::KeyReleased {
                        key: iced_key.clone(),
                        modified_key: iced_key,
                        physical_key,
                        modifiers: *modifiers,
                        location: keyboard::Location::Standard,
                    }))
                }
                _ => None,
            }
        }
        WindowEvent::MouseWheel { delta, .. } => {
            // Without this the scrollables never move: every other
            // arm maps, but the wheel fell into the catch-all below.
            let delta = match delta {
                tao::event::MouseScrollDelta::LineDelta(x, y) => mouse::ScrollDelta::Lines { x, y },
                tao::event::MouseScrollDelta::PixelDelta(position) => {
                    let logical = position.to_logical::<f64>(scale_factor as f64);
                    mouse::ScrollDelta::Pixels {
                        x: logical.x as f32,
                        y: logical.y as f32,
                    }
                }
                // Non-exhaustive upstream enum: unknown future
                // deltas scroll nothing instead of breaking the build.
                _ => return None,
            };
            Some(Event::Mouse(mouse::Event::WheelScrolled { delta }))
        }
        WindowEvent::MouseInput { button, state, .. } => {
            let btn = match button {
                tao::event::MouseButton::Left => mouse::Button::Left,
                tao::event::MouseButton::Right => mouse::Button::Right,
                tao::event::MouseButton::Middle => mouse::Button::Middle,
                _ => return None,
            };
            match state {
                tao::event::ElementState::Pressed => {
                    Some(Event::Mouse(mouse::Event::ButtonPressed(btn)))
                }
                tao::event::ElementState::Released => {
                    Some(Event::Mouse(mouse::Event::ButtonReleased(btn)))
                }
                _ => None,
            }
        }
        WindowEvent::ModifiersChanged(state) => {
            *modifiers = tao_modifiers_to_iced(state);
            None
        }
        WindowEvent::CloseRequested => {
            should_close.store(true, std::sync::atomic::Ordering::Release);
            None
        }
        // Touch, IME, axis motion, and other platform-specific events
        // are not needed for utility windows.
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_keycodes_have_mapping() {
        use tao::keyboard::KeyCode as T;
        let codes = [
            T::KeyA,
            T::KeyB,
            T::KeyC,
            T::KeyD,
            T::KeyE,
            T::KeyF,
            T::KeyG,
            T::KeyH,
            T::KeyI,
            T::KeyJ,
            T::KeyK,
            T::KeyL,
            T::KeyM,
            T::KeyN,
            T::KeyO,
            T::KeyP,
            T::KeyQ,
            T::KeyR,
            T::KeyS,
            T::KeyT,
            T::KeyU,
            T::KeyV,
            T::KeyW,
            T::KeyX,
            T::KeyY,
            T::KeyZ,
            T::Digit0,
            T::Digit1,
            T::Digit2,
            T::Digit3,
            T::Digit4,
            T::Digit5,
            T::Digit6,
            T::Digit7,
            T::Digit8,
            T::Digit9,
            T::ArrowUp,
            T::ArrowDown,
            T::ArrowLeft,
            T::ArrowRight,
            T::Enter,
            T::Escape,
            T::Space,
            T::Tab,
            T::Backspace,
            T::Delete,
            T::Home,
            T::End,
            T::PageUp,
            T::PageDown,
            T::Insert,
            T::F1,
            T::F2,
            T::F3,
            T::F4,
            T::F5,
            T::F6,
            T::F7,
            T::F8,
            T::F9,
            T::F10,
            T::F11,
            T::F12,
            T::ShiftLeft,
            T::ShiftRight,
            T::ControlLeft,
            T::ControlRight,
            T::AltLeft,
            T::AltRight,
            T::SuperLeft,
            T::SuperRight,
            T::Numpad0,
            T::Numpad1,
            T::Numpad5,
            T::Numpad9,
            T::NumpadAdd,
            T::NumpadSubtract,
            T::NumpadMultiply,
            T::NumpadDivide,
            T::NumpadDecimal,
            T::NumpadEnter,
            T::CapsLock,
            T::NumLock,
            T::ScrollLock,
            T::Comma,
            T::Period,
            T::Semicolon,
            T::Quote,
            T::Minus,
            T::Equal,
            T::BracketLeft,
            T::BracketRight,
            T::Backslash,
            T::Slash,
            T::IntlBackslash,
            T::Pause,
            T::PrintScreen,
            T::F13,
            T::F14,
            T::F15,
            T::F16,
            T::F17,
            T::F18,
            T::F19,
            T::F20,
            T::F21,
            T::F22,
            T::F23,
            T::F24,
            T::F25,
            T::F26,
            T::F27,
            T::F28,
            T::F29,
            T::F30,
            T::F31,
            T::F32,
            T::F33,
            T::F34,
            T::F35,
            T::Convert,
            T::NonConvert,
            T::Lang1,
            T::Lang2,
            T::Lang3,
            T::Lang4,
            T::Lang5,
            T::LaunchApp1,
            T::LaunchApp2,
            T::LaunchMail,
            T::Fn,
            T::FnLock,
            T::Hyper,
            T::Turbo,
            T::Sleep,
            T::Power,
            T::WakeUp,
            T::Suspend,
            T::AudioVolumeMute,
            T::AudioVolumeDown,
            T::AudioVolumeUp,
            T::MediaPlayPause,
            T::MediaStop,
            T::MediaTrackNext,
            T::MediaTrackPrevious,
            T::MediaSelect,
            T::BrowserBack,
            T::BrowserForward,
            T::BrowserRefresh,
            T::BrowserStop,
            T::BrowserSearch,
            T::BrowserHome,
            T::BrowserFavorites,
            T::Copy,
            T::Cut,
            T::Paste,
            T::Undo,
            T::Find,
            T::Help,
            T::Props,
            T::Select,
            T::Open,
            T::Resume,
            T::Eject,
            T::Abort,
            T::Again,
            T::ContextMenu,
            T::KanaMode,
            T::Katakana,
            T::Hiragana,
            T::IntlRo,
            T::IntlYen,
            T::Numpad2,
            T::Numpad3,
            T::Numpad4,
            T::Numpad6,
            T::Numpad7,
            T::Numpad8,
            T::NumpadComma,
            T::NumpadBackspace,
            T::NumpadClear,
            T::NumpadClearEntry,
            T::NumpadHash,
            T::NumpadEqual,
            T::NumpadStar,
            T::NumpadParenLeft,
            T::NumpadParenRight,
            T::NumpadMemoryStore,
            T::NumpadMemoryRecall,
            T::NumpadMemoryClear,
            T::NumpadMemoryAdd,
            T::NumpadMemorySubtract,
            T::Plus,
        ];
        for code in codes {
            let iced_code = tao_keycode_to_iced_code(code);
            // Backquote is the fallback; no tested key should hit it.
            assert_ne!(
                iced_code as i32,
                keyboard::key::Code::Backquote as i32,
                "KeyCode variant {:?} fell through to fallback",
                code,
            );
        }
    }

    #[test]
    fn character_key_round_trip() {
        let key = tao::keyboard::Key::Character("a");
        let iced = tao_key_to_iced_key(&key);
        assert_eq!(iced, keyboard::Key::Character(SmolStr::new("a")));
    }

    #[test]
    fn empty_string_character_preserved() {
        let key = tao::keyboard::Key::Character("");
        assert_eq!(
            tao_key_to_iced_key(&key),
            keyboard::Key::Character(SmolStr::new(""))
        );
    }

    #[test]
    fn window_events_convert_or_pass_through() {
        use std::sync::atomic::AtomicBool;
        use tao::event::{DeviceId, ElementState, WindowEvent};
        let mut cursor = mouse::Cursor::default();
        let mut modifiers = keyboard::Modifiers::default();
        let should_close = AtomicBool::new(false);
        fn convert(
            event: WindowEvent,
            cursor: &mut mouse::Cursor,
            modifiers: &mut keyboard::Modifiers,
            should_close: &AtomicBool,
        ) -> Option<iced::Event> {
            convert_tao_window_event(event, cursor, 1.0, modifiers, should_close)
        }
        macro_rules! feed {
            ($event:expr) => {
                convert($event, &mut cursor, &mut modifiers, &should_close)
            };
        }
        // SAFETY: test-only `DeviceId::dummy()`; no transmute in production.
        let device = unsafe { DeviceId::dummy() };
        // Cursor movement tracks the pointer.
        let moved = feed!(WindowEvent::CursorMoved {
            device_id: device,
            position: tao::dpi::PhysicalPosition::new(10.0, 20.0),
            modifiers: tao::keyboard::ModifiersState::empty(),
        });
        assert!(matches!(
            moved,
            Some(iced::Event::Mouse(mouse::Event::CursorMoved { .. }))
        ));
        assert!(matches!(cursor, mouse::Cursor::Available(_)));
        // Leaving the window drops the cursor.
        assert!(feed!(WindowEvent::CursorLeft { device_id: device }).is_none());
        assert!(matches!(cursor, mouse::Cursor::Unavailable));
        // Mouse buttons map left/right/middle, others pass through.
        for button in [
            tao::event::MouseButton::Left,
            tao::event::MouseButton::Right,
            tao::event::MouseButton::Middle,
        ] {
            let click = feed!(WindowEvent::MouseInput {
                device_id: device,
                state: ElementState::Pressed,
                button,
                modifiers: tao::keyboard::ModifiersState::empty(),
            });
            assert!(matches!(
                click,
                Some(iced::Event::Mouse(mouse::Event::ButtonPressed(_)))
            ));
        }
        let other = feed!(WindowEvent::MouseInput {
            device_id: device,
            state: ElementState::Pressed,
            button: tao::event::MouseButton::Other(9),
            modifiers: tao::keyboard::ModifiersState::empty(),
        });
        assert!(other.is_none());
        // Modifiers update state; close flips the flag.
        let changed = feed!(WindowEvent::ModifiersChanged(
            tao::keyboard::ModifiersState::empty(),
        ));
        assert!(changed.is_none());
        // Wheel deltas reach the scrollables (the reported bug was
        // the wheel falling into the catch-all below).
        let lines = feed!(WindowEvent::MouseWheel {
            device_id: device,
            delta: tao::event::MouseScrollDelta::LineDelta(0.0, 1.0),
            phase: tao::event::TouchPhase::Moved,
            modifiers: tao::keyboard::ModifiersState::empty(),
        });
        assert!(matches!(
            lines,
            Some(iced::Event::Mouse(mouse::Event::WheelScrolled {
                delta: mouse::ScrollDelta::Lines { .. }
            }))
        ));
        let pixels = feed!(WindowEvent::MouseWheel {
            device_id: device,
            delta: tao::event::MouseScrollDelta::PixelDelta(tao::dpi::PhysicalPosition::new(
                0.0, 24.0
            )),
            phase: tao::event::TouchPhase::Moved,
            modifiers: tao::keyboard::ModifiersState::empty(),
        });
        assert!(matches!(
            pixels,
            Some(iced::Event::Mouse(mouse::Event::WheelScrolled {
                delta: mouse::ScrollDelta::Pixels { .. }
            }))
        ));
        let released = feed!(WindowEvent::MouseInput {
            device_id: device,
            state: ElementState::Released,
            button: tao::event::MouseButton::Left,
            modifiers: tao::keyboard::ModifiersState::empty(),
        });
        assert!(matches!(
            released,
            Some(iced::Event::Mouse(mouse::Event::ButtonReleased(_)))
        ));
        assert!(feed!(WindowEvent::CloseRequested).is_none());
        assert!(should_close.load(std::sync::atomic::Ordering::Acquire));
        // Anything else (resizes and friends) passes through silently.
        assert!(feed!(WindowEvent::Resized(tao::dpi::PhysicalSize::new(800, 600))).is_none());
    }

    #[test]
    fn modifiers_map_each_flag() {
        use tao::keyboard::ModifiersState as M;
        assert!(tao_modifiers_to_iced(M::empty()).is_empty());
        let shift = tao_modifiers_to_iced(M::SHIFT);
        assert!(shift.contains(keyboard::Modifiers::SHIFT));
        assert!(!shift.contains(keyboard::Modifiers::CTRL));
        let ctrl = tao_modifiers_to_iced(M::CONTROL);
        assert!(ctrl.contains(keyboard::Modifiers::CTRL));
        assert!(!ctrl.contains(keyboard::Modifiers::ALT));
        let alt = tao_modifiers_to_iced(M::ALT);
        assert!(alt.contains(keyboard::Modifiers::ALT));
        let logo = tao_modifiers_to_iced(M::SUPER);
        assert!(logo.contains(keyboard::Modifiers::LOGO));
        let all = tao_modifiers_to_iced(M::SHIFT | M::CONTROL | M::ALT | M::SUPER);
        for flag in [
            keyboard::Modifiers::SHIFT,
            keyboard::Modifiers::CTRL,
            keyboard::Modifiers::ALT,
            keyboard::Modifiers::LOGO,
        ] {
            assert!(all.contains(flag));
        }
    }

    #[test]
    fn named_editing_keys_map_explicitly() {
        use iced::keyboard::key::Named;
        use tao::keyboard::Key as T;
        for (tao_key, named) in [
            (T::Backspace, Named::Backspace),
            (T::Delete, Named::Delete),
            (T::Enter, Named::Enter),
            (T::Escape, Named::Escape),
            (T::Tab, Named::Tab),
        ] {
            assert_eq!(tao_key_to_iced_key(&tao_key), keyboard::Key::Named(named));
        }
        assert_eq!(
            tao_key_to_iced_key(&tao::keyboard::Key::F1),
            keyboard::Key::Unidentified
        );
        // Non-editing named keys stay unidentified: physical codes
        // (not logical names) drive shortcuts.
        assert_eq!(
            tao_key_to_iced_key(&tao::keyboard::Key::Space),
            keyboard::Key::Unidentified
        );
        assert_eq!(
            tao_key_to_iced_key(&tao::keyboard::Key::Shift),
            keyboard::Key::Unidentified
        );
    }

    #[test]
    fn default_font_matches_platform() {
        #[cfg(target_os = "windows")]
        assert_eq!(default_font(), Font::with_name("Yu Gothic UI"));
        #[cfg(target_os = "macos")]
        assert_eq!(default_font(), Font::with_name("Hiragino Sans"));
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        assert_eq!(default_font(), Font::DEFAULT);
    }

    #[test]
    fn backquote_maps_to_itself_not_fallback() {
        use tao::keyboard::KeyCode as T;
        assert_eq!(keycode_to_key(T::Backquote), Some(Key::Backquote));
    }

    #[test]
    fn unmapped_codes_fall_back_to_backquote() {
        let code = tao::keyboard::KeyCode::Unidentified(tao::keyboard::NativeKeyCode::Unidentified);
        assert!(keycode_to_key(code).is_none());
        assert_eq!(
            tao_keycode_to_iced_code(code) as i32,
            keyboard::key::Code::Backquote as i32
        );
    }
}

//! Domain-to-iced conversion: only domain types in, only iced events out.
//! Tao-to-domain mapping lives in `crate::aux::tao_map`.

use iced::{Font, keyboard};

use crate::aux::domain::{AuxButton, AuxModifiers};
pub(crate) use crate::aux::tao_map::keycode_to_key;

/// Map a domain button to its iced counterpart.
pub(crate) fn aux_button_to_iced(button: AuxButton) -> iced::mouse::Button {
    match button {
        AuxButton::Left => iced::mouse::Button::Left,
        AuxButton::Right => iced::mouse::Button::Right,
        AuxButton::Middle => iced::mouse::Button::Middle,
    }
}

/// Map domain modifiers to iced modifiers.
pub(crate) fn aux_modifiers_to_iced(m: AuxModifiers) -> keyboard::Modifiers {
    let mut out = keyboard::Modifiers::empty();
    out.set(keyboard::Modifiers::SHIFT, m.shift());
    out.set(keyboard::Modifiers::CTRL, m.ctrl());
    out.set(keyboard::Modifiers::ALT, m.alt());
    out.set(keyboard::Modifiers::LOGO, m.logo());
    out
}

/// Map a domain key plus produced text to an iced logical key.
pub(crate) fn domain_key_to_iced(key: &nerust_keyboard::Key, text: Option<&str>) -> keyboard::Key {
    use iced::keyboard::key::Named;
    use nerust_keyboard::Key as K;
    if let Some(text) = text {
        return keyboard::Key::Character(iced_winit::core::SmolStr::new(text));
    }
    match key {
        K::Backspace => keyboard::Key::Named(Named::Backspace),
        K::Delete => keyboard::Key::Named(Named::Delete),
        K::Enter => keyboard::Key::Named(Named::Enter),
        K::Escape => keyboard::Key::Named(Named::Escape),
        K::Tab => keyboard::Key::Named(Named::Tab),
        _ => keyboard::Key::Unidentified,
    }
}

/// Feed one domain event, tracking cursor, modifiers, and close.
/// Pure over caller-owned state: the shell passes its live fields.
#[allow(clippy::too_many_arguments)]
pub(crate) fn feed_domain_event(
    event: crate::aux::domain::AuxNativeEvent,
    cursor: &mut iced::mouse::Cursor,
    modifiers: &mut keyboard::Modifiers,
    scale_factor: f32,
    should_close: &std::sync::atomic::AtomicBool,
) -> Option<iced::Event> {
    use crate::aux::domain::AuxNativeEvent as E;
    match event {
        E::CursorMoved(x, y) => {
            let point = iced::Point::new(x as f32 / scale_factor, y as f32 / scale_factor);
            *cursor = iced::mouse::Cursor::Available(point);
            Some(iced::Event::Mouse(iced::mouse::Event::CursorMoved {
                position: point,
            }))
        }
        E::CursorLeft => {
            *cursor = iced::mouse::Cursor::Unavailable;
            None
        }
        E::KeyPressed { key, text, repeat } => {
            let logical = domain_key_to_iced(&key, text.as_deref());
            let produced = text.map(iced_winit::core::SmolStr::new);
            Some(iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                key: logical.clone(),
                modified_key: logical,
                physical_key: iced::keyboard::key::Physical::Code(key.into()),
                modifiers: *modifiers,
                location: iced::keyboard::Location::Standard,
                text: produced,
                repeat,
            }))
        }
        E::KeyReleased { key } => {
            let logical = domain_key_to_iced(&key, None);
            Some(iced::Event::Keyboard(iced::keyboard::Event::KeyReleased {
                key: logical.clone(),
                modified_key: logical,
                physical_key: iced::keyboard::key::Physical::Code(key.into()),
                modifiers: *modifiers,
                location: iced::keyboard::Location::Standard,
            }))
        }
        // Iced 0.14 has no IME-commit event: the domain still carries
        // `TextCommit` (see `adapt_event`), conversion drops it here.
        E::TextCommit(_) => None,
        E::MousePressed(button) => {
            let button = aux_button_to_iced(button);
            Some(iced::Event::Mouse(iced::mouse::Event::ButtonPressed(
                button,
            )))
        }
        E::MouseReleased(button) => {
            let button = aux_button_to_iced(button);
            Some(iced::Event::Mouse(iced::mouse::Event::ButtonReleased(
                button,
            )))
        }
        E::WheelLines(x, y) => Some(iced::Event::Mouse(iced::mouse::Event::WheelScrolled {
            delta: iced::mouse::ScrollDelta::Lines { x, y },
        })),
        E::WheelPixels(x, y) => Some(iced::Event::Mouse(iced::mouse::Event::WheelScrolled {
            delta: iced::mouse::ScrollDelta::Pixels {
                x: x / scale_factor,
                y: y / scale_factor,
            },
        })),
        E::ModifiersChanged(next) => {
            *modifiers = aux_modifiers_to_iced(next);
            None
        }
        E::CloseRequested => {
            should_close.store(true, std::sync::atomic::Ordering::Release);
            None
        }
        // Focus, moves, resizes, and unmapped input never reach the program.
        _ => None,
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

#[cfg(test)]
mod tests {
    use super::*;
    use nerust_keyboard::Key;

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
            assert!(
                keycode_to_key(code).is_some(),
                "KeyCode variant {:?} fell through to fallback",
                code,
            );
        }
    }

    #[test]
    fn domain_feed_scales_by_live_scale() {
        use iced::mouse;
        use std::sync::atomic::AtomicBool;
        let mut cursor = mouse::Cursor::default();
        let mut modifiers = keyboard::Modifiers::default();
        let should_close = AtomicBool::new(false);
        // Physical pixels divide by the live scale, like the shell feed.
        let moved = feed_domain_event(
            crate::aux::domain::AuxNativeEvent::CursorMoved(200.0, 100.0),
            &mut cursor,
            &mut modifiers,
            2.0,
            &should_close,
        );
        assert!(matches!(
            moved,
            Some(iced::Event::Mouse(mouse::Event::CursorMoved { position }))
                if position == iced::Point::new(100.0, 50.0)
        ));
    }

    #[test]
    fn produced_text_wins_over_physical_name() {
        assert_eq!(
            domain_key_to_iced(&Key::KeyA, Some("a")),
            keyboard::Key::Character(iced_winit::core::SmolStr::new("a"))
        );
        assert_eq!(
            domain_key_to_iced(&Key::KeyA, Some("あ")),
            keyboard::Key::Character(iced_winit::core::SmolStr::new("あ"))
        );
    }

    #[test]
    fn named_editing_keys_map_explicitly() {
        use iced::keyboard::key::Named;
        for (key, named) in [
            (Key::Backspace, Named::Backspace),
            (Key::Delete, Named::Delete),
            (Key::Enter, Named::Enter),
            (Key::Escape, Named::Escape),
            (Key::Tab, Named::Tab),
        ] {
            assert_eq!(domain_key_to_iced(&key, None), keyboard::Key::Named(named));
        }
        assert_eq!(
            domain_key_to_iced(&Key::F1, None),
            keyboard::Key::Unidentified
        );
        // Non-editing named keys stay unidentified: physical codes
        // (not logical names) drive shortcuts.
        assert_eq!(
            domain_key_to_iced(&Key::Space, None),
            keyboard::Key::Unidentified
        );
        assert_eq!(
            domain_key_to_iced(&Key::ShiftLeft, None),
            keyboard::Key::Unidentified
        );
    }

    #[test]
    fn domain_key_round_trips_through_iced_physical() {
        // The exhaustive physical table lives on `Key`: iced codes
        // convert back without loss (except the known Plus gap).
        for key in [Key::KeyA, Key::Enter, Key::F4, Key::Numpad9, Key::Hiragana] {
            let code: iced::keyboard::key::Code = key.into();
            let physical = iced::keyboard::key::Physical::Code(code);
            let round_trip: Key = physical.try_into().expect("round trip");
            assert_eq!(key, round_trip);
        }
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
    fn mapped_buttons_cross_but_others_drop() {
        use crate::aux::native::adapt_event;
        use tao::event::{DeviceId, ElementState, MouseButton, WindowEvent};
        // SAFETY: test-only `DeviceId::dummy()`; no transmute in production.
        let device = unsafe { DeviceId::dummy() };
        for button in [MouseButton::Left, MouseButton::Right, MouseButton::Middle] {
            let pressed = adapt_event(&WindowEvent::MouseInput {
                device_id: device,
                state: ElementState::Pressed,
                button,
                modifiers: tao::keyboard::ModifiersState::empty(),
            });
            assert!(matches!(
                pressed,
                crate::aux::domain::AuxNativeEvent::MousePressed(_)
            ));
        }
        let other = adapt_event(&WindowEvent::MouseInput {
            device_id: device,
            state: ElementState::Pressed,
            button: MouseButton::Other(9),
            modifiers: tao::keyboard::ModifiersState::empty(),
        });
        assert_eq!(other, crate::aux::domain::AuxNativeEvent::Other);
    }

    #[test]
    fn aux_buttons_map_to_each_iced_button() {
        assert_eq!(
            aux_button_to_iced(AuxButton::Left),
            iced::mouse::Button::Left
        );
        assert_eq!(
            aux_button_to_iced(AuxButton::Right),
            iced::mouse::Button::Right
        );
        assert_eq!(
            aux_button_to_iced(AuxButton::Middle),
            iced::mouse::Button::Middle
        );
    }

    #[test]
    fn domain_modifiers_map_each_flag() {
        use crate::aux::tao_map::tao_modifiers_to_domain;
        use tao::keyboard::ModifiersState as M;
        assert!(aux_modifiers_to_iced(tao_modifiers_to_domain(M::empty())).is_empty());
        let shift = aux_modifiers_to_iced(tao_modifiers_to_domain(M::SHIFT));
        assert!(shift.contains(keyboard::Modifiers::SHIFT));
        assert!(!shift.contains(keyboard::Modifiers::CTRL));
        let ctrl = aux_modifiers_to_iced(tao_modifiers_to_domain(M::CONTROL));
        assert!(ctrl.contains(keyboard::Modifiers::CTRL));
        let alt = aux_modifiers_to_iced(tao_modifiers_to_domain(M::ALT));
        assert!(alt.contains(keyboard::Modifiers::ALT));
        let logo = aux_modifiers_to_iced(tao_modifiers_to_domain(M::SUPER));
        assert!(logo.contains(keyboard::Modifiers::LOGO));
        let all = aux_modifiers_to_iced(tao_modifiers_to_domain(
            M::SHIFT | M::CONTROL | M::ALT | M::SUPER,
        ));
        for flag in [
            keyboard::Modifiers::SHIFT,
            keyboard::Modifiers::CTRL,
            keyboard::Modifiers::ALT,
            keyboard::Modifiers::LOGO,
        ] {
            assert!(all.contains(flag));
        }
    }

    fn feed_harness() -> (
        iced::mouse::Cursor,
        keyboard::Modifiers,
        std::sync::atomic::AtomicBool,
    ) {
        (
            iced::mouse::Cursor::default(),
            keyboard::Modifiers::empty(),
            std::sync::atomic::AtomicBool::new(false),
        )
    }

    #[test]
    fn feed_maps_key_press_and_release() {
        use crate::aux::domain::AuxNativeEvent as E;
        let (mut cursor, mut modifiers, should_close) = feed_harness();
        // Text plus repeat ride into the iced key event.
        let pressed = feed_domain_event(
            E::KeyPressed {
                key: Key::KeyA,
                text: Some("a".to_string()),
                repeat: true,
            },
            &mut cursor,
            &mut modifiers,
            1.0,
            &should_close,
        );
        assert!(matches!(
            pressed,
            Some(iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                repeat: true,
                ..
            }))
        ));
        // Missing text falls back to the named table; release carries no text.
        let no_text = feed_domain_event(
            E::KeyPressed {
                key: Key::Enter,
                text: None,
                repeat: false,
            },
            &mut cursor,
            &mut modifiers,
            1.0,
            &should_close,
        );
        assert!(matches!(
            no_text,
            Some(iced::Event::Keyboard(
                iced::keyboard::Event::KeyPressed { .. }
            ))
        ));
        let released = feed_domain_event(
            E::KeyReleased { key: Key::Enter },
            &mut cursor,
            &mut modifiers,
            1.0,
            &should_close,
        );
        assert!(matches!(
            released,
            Some(iced::Event::Keyboard(
                iced::keyboard::Event::KeyReleased { .. }
            ))
        ));
        assert!(!should_close.load(std::sync::atomic::Ordering::Acquire));
    }

    #[test]
    fn feed_drops_text_commit_without_iced_counterpart() {
        use crate::aux::domain::AuxNativeEvent as E;
        let (mut cursor, mut modifiers, should_close) = feed_harness();
        assert!(
            feed_domain_event(
                E::TextCommit("あ".to_string()),
                &mut cursor,
                &mut modifiers,
                1.0,
                &should_close,
            )
            .is_none()
        );
    }

    #[test]
    fn feed_maps_each_mouse_button_both_edges() {
        use crate::aux::domain::AuxNativeEvent as E;
        let (mut cursor, mut modifiers, should_close) = feed_harness();
        for (domain, iced_button) in [
            (AuxButton::Left, iced::mouse::Button::Left),
            (AuxButton::Right, iced::mouse::Button::Right),
            (AuxButton::Middle, iced::mouse::Button::Middle),
        ] {
            let pressed = feed_domain_event(
                E::MousePressed(domain),
                &mut cursor,
                &mut modifiers,
                1.0,
                &should_close,
            );
            assert!(matches!(
                pressed,
                Some(iced::Event::Mouse(iced::mouse::Event::ButtonPressed(b)))
                    if b == iced_button
            ));
            let released = feed_domain_event(
                E::MouseReleased(domain),
                &mut cursor,
                &mut modifiers,
                1.0,
                &should_close,
            );
            assert!(matches!(
                released,
                Some(iced::Event::Mouse(iced::mouse::Event::ButtonReleased(b)))
                    if b == iced_button
            ));
        }
    }

    #[test]
    fn feed_scales_wheel_pixels_and_passes_lines() {
        use crate::aux::domain::AuxNativeEvent as E;
        let (mut cursor, mut modifiers, should_close) = feed_harness();
        let lines = feed_domain_event(
            E::WheelLines(0.0, 3.0),
            &mut cursor,
            &mut modifiers,
            2.0,
            &should_close,
        );
        assert!(matches!(
            lines,
            Some(iced::Event::Mouse(iced::mouse::Event::WheelScrolled {
                delta: iced::mouse::ScrollDelta::Lines { y, .. }
            })) if y == 3.0
        ));
        // Pixel deltas divide by the live scale like cursor moves.
        let pixels = feed_domain_event(
            E::WheelPixels(0.0, 24.0),
            &mut cursor,
            &mut modifiers,
            2.0,
            &should_close,
        );
        assert!(matches!(
            pixels,
            Some(iced::Event::Mouse(iced::mouse::Event::WheelScrolled {
                delta: iced::mouse::ScrollDelta::Pixels { y, .. }
            })) if y == 12.0
        ));
    }

    #[test]
    fn feed_tracks_modifiers_and_close() {
        use crate::aux::domain::{AuxModifiers, AuxNativeEvent as E};
        let (mut cursor, mut modifiers, should_close) = feed_harness();
        assert!(
            feed_domain_event(
                E::ModifiersChanged(AuxModifiers::empty()),
                &mut cursor,
                &mut modifiers,
                1.0,
                &should_close,
            )
            .is_none()
        );
        assert_eq!(modifiers, keyboard::Modifiers::empty());
        assert!(
            feed_domain_event(
                E::CloseRequested,
                &mut cursor,
                &mut modifiers,
                1.0,
                &should_close,
            )
            .is_none()
        );
        assert!(should_close.load(std::sync::atomic::Ordering::Acquire));
    }

    #[test]
    fn feed_ignores_non_program_events() {
        use crate::aux::domain::AuxNativeEvent as E;
        let (mut cursor, mut modifiers, should_close) = feed_harness();
        for event in [
            E::Focused(true),
            E::Focused(false),
            E::Destroyed,
            E::Moved(1, 2),
            E::Resized(800, 600),
            E::ScaleChanged(2.0),
            E::Other,
            E::TextCommit("あ".to_string()),
        ] {
            assert!(
                feed_domain_event(event, &mut cursor, &mut modifiers, 1.0, &should_close).is_none()
            );
        }
        // Cursor-left drops the pointer without an event.
        assert!(
            feed_domain_event(
                E::CursorLeft,
                &mut cursor,
                &mut modifiers,
                1.0,
                &should_close
            )
            .is_none()
        );
        assert!(matches!(cursor, iced::mouse::Cursor::Unavailable));
        assert!(!should_close.load(std::sync::atomic::Ordering::Acquire));
    }

    #[test]
    fn window_events_convert_or_pass_through() {
        use crate::aux::native::adapt_event;
        use iced::mouse;
        use std::sync::atomic::AtomicBool;
        use tao::event::{DeviceId, ElementState, WindowEvent};
        let mut cursor = mouse::Cursor::default();
        let mut modifiers = keyboard::Modifiers::default();
        let should_close = AtomicBool::new(false);
        macro_rules! feed {
            ($event:expr) => {{
                let domain = adapt_event(&$event);
                feed_domain_event(domain, &mut cursor, &mut modifiers, 1.0, &should_close)
            }};
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
        // IME text arrives as its own event, dropped by conversion.
        assert!(feed!(WindowEvent::ReceivedImeText("あ".to_string())).is_none());
    }
}

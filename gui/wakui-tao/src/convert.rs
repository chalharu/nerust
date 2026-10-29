//! Domain-to-iced conversion: only domain types in, only iced events out.
//! Tao-to-domain mapping lives in `crate::tao_map`.

use iced::{Font, keyboard};

use crate::domain::{AuxButton, AuxModifiers};
pub use crate::tao_map::keycode_to_key;

/// Map a domain button to its iced counterpart.
pub fn aux_button_to_iced(button: AuxButton) -> iced::mouse::Button {
    match button {
        AuxButton::Left => iced::mouse::Button::Left,
        AuxButton::Right => iced::mouse::Button::Right,
        AuxButton::Middle => iced::mouse::Button::Middle,
    }
}

/// Map domain modifiers to iced modifiers.
pub fn aux_modifiers_to_iced(m: AuxModifiers) -> keyboard::Modifiers {
    let mut out = keyboard::Modifiers::empty();
    out.set(keyboard::Modifiers::SHIFT, m.shift());
    out.set(keyboard::Modifiers::CTRL, m.ctrl());
    out.set(keyboard::Modifiers::ALT, m.alt());
    out.set(keyboard::Modifiers::LOGO, m.logo());
    out
}

/// Map a domain key plus produced text to an iced logical key.
/// Control-carrying text never masks editing keys: tao reports Enter as
/// `"\r"`, so `Named` wins whenever the key already has an identity.
pub fn domain_key_to_iced(key: &wakui_core::Key, text: Option<&str>) -> keyboard::Key {
    use iced::keyboard::key::Named;
    use wakui_core::Key as K;
    let named = match key {
        K::Backspace => Some(Named::Backspace),
        K::Delete => Some(Named::Delete),
        K::Enter | K::NumpadEnter => Some(Named::Enter),
        K::Escape => Some(Named::Escape),
        K::Tab => Some(Named::Tab),
        // Caret and paging keys: aux text fields need logical identity,
        // shortcuts keep reading the physical code.
        K::ArrowUp => Some(Named::ArrowUp),
        K::ArrowDown => Some(Named::ArrowDown),
        K::ArrowLeft => Some(Named::ArrowLeft),
        K::ArrowRight => Some(Named::ArrowRight),
        K::Home => Some(Named::Home),
        K::End => Some(Named::End),
        K::PageUp => Some(Named::PageUp),
        K::PageDown => Some(Named::PageDown),
        K::Insert => Some(Named::Insert),
        K::Space => Some(Named::Space),
        K::F1 => Some(Named::F1),
        K::F2 => Some(Named::F2),
        K::F3 => Some(Named::F3),
        K::F4 => Some(Named::F4),
        K::F5 => Some(Named::F5),
        K::F6 => Some(Named::F6),
        K::F7 => Some(Named::F7),
        K::F8 => Some(Named::F8),
        K::F9 => Some(Named::F9),
        K::F10 => Some(Named::F10),
        K::F11 => Some(Named::F11),
        K::F12 => Some(Named::F12),
        _ => None,
    };
    if let Some(named) = named {
        return keyboard::Key::Named(named);
    }
    match text {
        Some(text) if !text.is_empty() && text.chars().all(|c| !c.is_control()) => {
            keyboard::Key::Character(iced_winit::core::SmolStr::new(text))
        }
        _ => keyboard::Key::Unidentified,
    }
}

/// Owned feed state: cursor, modifiers, scale, and close in one borrow.
/// Callers pass their live fields once instead of threading four params.
pub struct FeedContext<'a> {
    pub cursor: &'a mut iced::mouse::Cursor,
    pub modifiers: &'a mut keyboard::Modifiers,
    pub scale_factor: f32,
    pub should_close: &'a std::sync::atomic::AtomicBool,
}

/// Feed one domain event, tracking cursor, modifiers, and close.
/// Pure over caller-owned state: the shell passes its live [`FeedContext`].
pub fn feed_domain_event(
    event: crate::domain::AuxNativeEvent,
    ctx: &mut FeedContext<'_>,
) -> Option<iced::Event> {
    use crate::domain::AuxNativeEvent as E;
    let scale_factor = ctx.scale_factor;
    let should_close = ctx.should_close;
    let (cursor, modifiers) = (&mut *ctx.cursor, &mut *ctx.modifiers);
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
                physical_key: iced::keyboard::key::Physical::Code(crate::keyboard::code_from_key(
                    key,
                )),
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
                physical_key: iced::keyboard::key::Physical::Code(crate::keyboard::code_from_key(
                    key,
                )),
                modifiers: *modifiers,
                location: iced::keyboard::Location::Standard,
            }))
        }
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
/// Language-specific needs live view-side; the backend only needs a font.
pub fn default_font() -> Font {
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
    use wakui_core::Key;

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
            crate::domain::AuxNativeEvent::CursorMoved(200.0, 100.0),
            &mut FeedContext {
                cursor: &mut cursor,
                modifiers: &mut modifiers,
                scale_factor: 2.0,
                should_close: &should_close,
            },
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
    fn control_text_does_not_mask_editing_keys() {
        use iced::keyboard::key::Named;
        for (key, text, named) in [
            (Key::Enter, "\r", Named::Enter),
            (Key::Backspace, "\u{8}", Named::Backspace),
            (Key::Tab, "\t", Named::Tab),
            (Key::NumpadEnter, "\r", Named::Enter),
        ] {
            assert_eq!(
                domain_key_to_iced(&key, Some(text)),
                keyboard::Key::Named(named)
            );
        }
        assert_eq!(
            domain_key_to_iced(&Key::NumpadEnter, None),
            keyboard::Key::Named(Named::Enter)
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
            (Key::ArrowUp, Named::ArrowUp),
            (Key::ArrowDown, Named::ArrowDown),
            (Key::ArrowLeft, Named::ArrowLeft),
            (Key::ArrowRight, Named::ArrowRight),
            (Key::Home, Named::Home),
            (Key::End, Named::End),
            (Key::PageUp, Named::PageUp),
            (Key::PageDown, Named::PageDown),
            (Key::Insert, Named::Insert),
            (Key::Space, Named::Space),
            (Key::F1, Named::F1),
            (Key::F6, Named::F6),
            (Key::F12, Named::F12),
        ] {
            assert_eq!(domain_key_to_iced(&key, None), keyboard::Key::Named(named));
        }
        // Separately tracked modifiers stay unidentified: iced reads them
        // from the modifiers state, not the logical key.
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
            let code = crate::keyboard::code_from_key(key);
            let physical = iced::keyboard::key::Physical::Code(code);
            let round_trip: Key = crate::keyboard::key_from_physical(physical).expect("round trip");
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
        use crate::domain::AuxNativeEvent;
        use crate::tao_map::tao_button_to_aux;
        use tao::event::MouseButton;
        for button in [MouseButton::Left, MouseButton::Right, MouseButton::Middle] {
            let pressed = tao_button_to_aux(button).map(AuxNativeEvent::MousePressed);
            assert!(matches!(pressed, Some(AuxNativeEvent::MousePressed(_))));
        }
        assert_eq!(tao_button_to_aux(MouseButton::Other(9)), None);
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
        use crate::tao_map::tao_modifiers_to_domain;
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
        use crate::domain::AuxNativeEvent as E;
        let (mut cursor, mut modifiers, should_close) = feed_harness();
        // Text plus repeat ride into the iced key event.
        let pressed = feed_domain_event(
            E::KeyPressed {
                key: Key::KeyA,
                text: Some("a".to_string()),
                repeat: true,
            },
            &mut FeedContext {
                cursor: &mut cursor,
                modifiers: &mut modifiers,
                scale_factor: 1.0,
                should_close: &should_close,
            },
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
            &mut FeedContext {
                cursor: &mut cursor,
                modifiers: &mut modifiers,
                scale_factor: 1.0,
                should_close: &should_close,
            },
        );
        assert!(matches!(
            no_text,
            Some(iced::Event::Keyboard(
                iced::keyboard::Event::KeyPressed { .. }
            ))
        ));
        let released = feed_domain_event(
            E::KeyReleased { key: Key::Enter },
            &mut FeedContext {
                cursor: &mut cursor,
                modifiers: &mut modifiers,
                scale_factor: 1.0,
                should_close: &should_close,
            },
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
    fn feed_drops_unmapped_domain_events() {
        use crate::domain::AuxNativeEvent as E;
        let (mut cursor, mut modifiers, should_close) = feed_harness();
        assert!(
            feed_domain_event(
                E::Other,
                &mut FeedContext {
                    cursor: &mut cursor,
                    modifiers: &mut modifiers,
                    scale_factor: 1.0,
                    should_close: &should_close
                }
            )
            .is_none()
        );
    }

    #[test]
    fn feed_maps_each_mouse_button_both_edges() {
        use crate::domain::AuxNativeEvent as E;
        let (mut cursor, mut modifiers, should_close) = feed_harness();
        for (domain, iced_button) in [
            (AuxButton::Left, iced::mouse::Button::Left),
            (AuxButton::Right, iced::mouse::Button::Right),
            (AuxButton::Middle, iced::mouse::Button::Middle),
        ] {
            let pressed = feed_domain_event(
                E::MousePressed(domain),
                &mut FeedContext {
                    cursor: &mut cursor,
                    modifiers: &mut modifiers,
                    scale_factor: 1.0,
                    should_close: &should_close,
                },
            );
            assert!(matches!(
                pressed,
                Some(iced::Event::Mouse(iced::mouse::Event::ButtonPressed(b)))
                    if b == iced_button
            ));
            let released = feed_domain_event(
                E::MouseReleased(domain),
                &mut FeedContext {
                    cursor: &mut cursor,
                    modifiers: &mut modifiers,
                    scale_factor: 1.0,
                    should_close: &should_close,
                },
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
        use crate::domain::AuxNativeEvent as E;
        let (mut cursor, mut modifiers, should_close) = feed_harness();
        let lines = feed_domain_event(
            E::WheelLines(0.0, 3.0),
            &mut FeedContext {
                cursor: &mut cursor,
                modifiers: &mut modifiers,
                scale_factor: 2.0,
                should_close: &should_close,
            },
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
            &mut FeedContext {
                cursor: &mut cursor,
                modifiers: &mut modifiers,
                scale_factor: 2.0,
                should_close: &should_close,
            },
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
        use crate::domain::{AuxModifiers, AuxNativeEvent as E};
        let (mut cursor, mut modifiers, should_close) = feed_harness();
        assert!(
            feed_domain_event(
                E::ModifiersChanged(AuxModifiers::empty()),
                &mut FeedContext {
                    cursor: &mut cursor,
                    modifiers: &mut modifiers,
                    scale_factor: 1.0,
                    should_close: &should_close,
                },
            )
            .is_none()
        );
        assert_eq!(modifiers, keyboard::Modifiers::empty());
        assert!(
            feed_domain_event(
                E::CloseRequested,
                &mut FeedContext {
                    cursor: &mut cursor,
                    modifiers: &mut modifiers,
                    scale_factor: 1.0,
                    should_close: &should_close,
                },
            )
            .is_none()
        );
        assert!(should_close.load(std::sync::atomic::Ordering::Acquire));
    }

    #[test]
    fn feed_ignores_non_program_events() {
        use crate::domain::AuxNativeEvent as E;
        let (mut cursor, mut modifiers, should_close) = feed_harness();
        for event in [
            E::Focused(true),
            E::Focused(false),
            E::Destroyed,
            E::Moved(1, 2),
            E::Resized(800, 600),
            E::ScaleChanged(2.0),
            E::Other,
        ] {
            assert!(
                feed_domain_event(
                    event,
                    &mut FeedContext {
                        cursor: &mut cursor,
                        modifiers: &mut modifiers,
                        scale_factor: 1.0,
                        should_close: &should_close
                    }
                )
                .is_none()
            );
        }
        // Cursor-left drops the pointer without an event.
        assert!(
            feed_domain_event(
                E::CursorLeft,
                &mut FeedContext {
                    cursor: &mut cursor,
                    modifiers: &mut modifiers,
                    scale_factor: 1.0,
                    should_close: &should_close,
                },
            )
            .is_none()
        );
        assert!(matches!(cursor, iced::mouse::Cursor::Unavailable));
        assert!(!should_close.load(std::sync::atomic::Ordering::Acquire));
    }

    #[test]
    fn window_events_convert_or_pass_through() {
        use crate::domain::{AuxButton, AuxModifiers, AuxNativeEvent as E};
        use iced::mouse;
        use std::sync::atomic::AtomicBool;
        let mut cursor = mouse::Cursor::default();
        let mut modifiers = keyboard::Modifiers::default();
        let should_close = AtomicBool::new(false);
        macro_rules! feed {
            ($domain:expr) => {{
                feed_domain_event(
                    $domain,
                    &mut FeedContext {
                        cursor: &mut cursor,
                        modifiers: &mut modifiers,
                        scale_factor: 1.0,
                        should_close: &should_close,
                    },
                )
            }};
        }
        // Cursor movement tracks the pointer.
        let moved = feed!(E::CursorMoved(10.0, 20.0));
        assert!(matches!(
            moved,
            Some(iced::Event::Mouse(mouse::Event::CursorMoved { .. }))
        ));
        assert!(matches!(cursor, mouse::Cursor::Available(_)));
        // Leaving the window drops the cursor.
        assert!(feed!(E::CursorLeft).is_none());
        assert!(matches!(cursor, mouse::Cursor::Unavailable));
        // Mouse buttons map left/right/middle, others pass through.
        for button in [AuxButton::Left, AuxButton::Right, AuxButton::Middle] {
            let click = feed!(E::MousePressed(button));
            assert!(matches!(
                click,
                Some(iced::Event::Mouse(mouse::Event::ButtonPressed(_)))
            ));
        }
        assert!(feed!(E::Other).is_none());
        // Modifiers update state; close flips the flag.
        let changed = feed!(E::ModifiersChanged(AuxModifiers::empty()));
        assert!(changed.is_none());
        // Wheel deltas reach the scrollables (the reported bug was
        // the wheel falling into the catch-all below).
        let lines = feed!(E::WheelLines(0.0, 1.0));
        assert!(matches!(
            lines,
            Some(iced::Event::Mouse(mouse::Event::WheelScrolled {
                delta: mouse::ScrollDelta::Lines { .. }
            }))
        ));
        let pixels = feed!(E::WheelPixels(0.0, 24.0));
        assert!(matches!(
            pixels,
            Some(iced::Event::Mouse(mouse::Event::WheelScrolled {
                delta: mouse::ScrollDelta::Pixels { .. }
            }))
        ));
        let released = feed!(E::MouseReleased(AuxButton::Left));
        assert!(matches!(
            released,
            Some(iced::Event::Mouse(mouse::Event::ButtonReleased(_)))
        ));
        assert!(feed!(E::CloseRequested).is_none());
        assert!(should_close.load(std::sync::atomic::Ordering::Acquire));
        // Anything else (resizes and friends) passes through silently.
        assert!(feed!(E::Other).is_none());
    }
}

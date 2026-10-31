use crate::{NativeWindow, NativeWindowEvent, Parent, TaoHost, View};
use panechord_runtime::{
    Binding, ImageFrame, KeyCode, Session, WindowEvent, WindowHooks, WindowHost, WindowSurface,
};
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle,
};
use std::{rc::Rc, sync::Arc};

struct Surface(Arc<NativeWindow>);

impl HasDisplayHandle for Surface {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        self.0.display_handle()
    }
}

impl HasWindowHandle for Surface {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        self.0.window_handle()
    }
}

impl WindowSurface for Surface {
    fn scale_factor(&self) -> f64 {
        self.0.scale_factor()
    }

    fn toggle_fullscreen(&self) {
        self.0.set_fullscreen(if self.0.fullscreen().is_some() {
            None
        } else {
            Some(crate::NativeFullscreen::Borderless(None))
        });
    }
}

impl WindowHost<View> for TaoHost {
    fn register_view(
        &self,
        session: &Session,
        id: &str,
        view: &View,
        hooks: WindowHooks,
    ) -> Result<(), String> {
        TaoHost::register_view(self, session, id, view, hooks)
    }
}

impl View {
    fn key_code(code: crate::NativeKeyCode) -> KeyCode {
        macro_rules! codes {
            ($($code:ident),+ $(,)?) => { match code { $(crate::NativeKeyCode::$code => KeyCode::$code,)+ _ => KeyCode::Unidentified } };
        }
        codes!(
            KeyA,
            KeyB,
            KeyC,
            KeyD,
            KeyE,
            KeyF,
            KeyG,
            KeyH,
            KeyI,
            KeyJ,
            KeyK,
            KeyL,
            KeyM,
            KeyN,
            KeyO,
            KeyP,
            KeyQ,
            KeyR,
            KeyS,
            KeyT,
            KeyU,
            KeyV,
            KeyW,
            KeyX,
            KeyY,
            KeyZ,
            Digit0,
            Digit1,
            Digit2,
            Digit3,
            Digit4,
            Digit5,
            Digit6,
            Digit7,
            Digit8,
            Digit9,
            F1,
            F2,
            F3,
            F4,
            F5,
            F6,
            F7,
            F8,
            F9,
            F10,
            F11,
            F12,
            F13,
            F14,
            F15,
            F16,
            F17,
            F18,
            F19,
            F20,
            F21,
            F22,
            F23,
            F24,
            F25,
            F26,
            F27,
            F28,
            F29,
            F30,
            F31,
            F32,
            F33,
            F34,
            F35,
            Enter,
            Escape,
            Space,
            Tab,
            Delete,
            Backspace,
            Insert,
            CapsLock,
            Home,
            End,
            PageUp,
            PageDown,
            ArrowUp,
            ArrowDown,
            ArrowLeft,
            ArrowRight,
            Minus,
            Equal,
            BracketLeft,
            BracketRight,
            Backslash,
            Semicolon,
            Quote,
            Backquote,
            Comma,
            Period,
            Slash,
            Numpad0,
            Numpad1,
            Numpad2,
            Numpad3,
            Numpad4,
            Numpad5,
            Numpad6,
            Numpad7,
            Numpad8,
            Numpad9,
            NumpadDecimal,
            NumpadAdd,
            NumpadSubtract,
            NumpadMultiply,
            NumpadDivide,
            NumpadEnter,
            NumpadEqual,
            ControlLeft,
            ControlRight,
            ShiftLeft,
            ShiftRight,
            AltLeft,
            AltRight,
            SuperLeft,
            SuperRight,
            PrintScreen,
            ScrollLock,
            Pause,
            NumLock,
            ContextMenu,
            AudioVolumeUp,
            AudioVolumeDown,
            AudioVolumeMute,
            MediaPlayPause,
            MediaStop,
            MediaTrackNext,
            MediaTrackPrevious,
            MediaSelect,
            BrowserBack,
            BrowserFavorites,
            BrowserForward,
            BrowserHome,
            BrowserRefresh,
            BrowserSearch,
            BrowserStop,
            Convert,
            NonConvert,
            KanaMode,
            IntlBackslash,
            IntlRo,
            IntlYen,
            Lang1,
            Lang2,
            Lang3,
            Lang4,
            Lang5,
        )
    }

    pub fn on_surface_ready(
        &self,
        callback: impl FnOnce(Rc<dyn WindowSurface>) -> Result<(), String> + 'static,
    ) -> Result<(), String> {
        self.on_native_window(move |window| callback(Rc::new(Surface(window))))
    }

    pub fn on_surface_viewport(&self, callback: Rc<dyn Fn(f32, f32)>) {
        self.on_native_viewport(callback);
    }

    pub fn on_surface_event(&self, callback: Rc<dyn Fn(&WindowEvent)>) {
        self.on_native_event(Rc::new(move |event| {
            let event = match event {
                NativeWindowEvent::Focused(focused) => WindowEvent::Focused(*focused),
                NativeWindowEvent::KeyboardInput {
                    event,
                    is_synthetic,
                    ..
                } => WindowEvent::KeyboardInput {
                    key: Self::key_code(event.physical_key),
                    pressed: event.state == crate::NativeElementState::Pressed,
                    repeat: event.repeat,
                    synthetic: *is_synthetic,
                },
                _ => return,
            };
            callback(&event);
        }));
    }

    pub fn surface(&self, parent: &Parent, id: &str, source: Binding<Option<ImageFrame>>) {
        let owner = Rc::downgrade(&self.0);
        self.mount_bound(
            parent,
            id,
            source,
            Rc::new(move |frame| {
                let (width, height) = owner
                    .upgrade()
                    .map_or((1.0, 1.0), |inner| inner.surface_bounds.get());
                element(frame, width, height)
            }),
        );
    }
}

#[cfg(test)]
mod input_tests {
    use super::*;
    #[test]
    fn native_tao_codes_map_directly_to_common_physical_keys() {
        for (native, common) in [
            (crate::NativeKeyCode::KeyV, KeyCode::KeyV),
            (crate::NativeKeyCode::ShiftRight, KeyCode::ShiftRight),
            (crate::NativeKeyCode::F35, KeyCode::F35),
            (crate::NativeKeyCode::IntlYen, KeyCode::IntlYen),
        ] {
            assert_eq!(View::key_code(native), common);
        }
    }
}

pub(super) fn element(frame: Option<ImageFrame>, width: f32, height: f32) -> crate::Element {
    use iced::{Length, widget};
    let content: crate::Element = if let Some(frame) = frame {
        widget::image(widget::image::Handle::from_rgba(
            frame.width(),
            frame.height(),
            frame.pixels().as_ref().to_vec(),
        ))
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    } else {
        widget::Space::new()
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    };
    widget::container(content)
        .width(width.max(1.0))
        .height(height.max(1.0))
        .style(|_| widget::container::Style {
            background: Some(iced::Color::BLACK.into()),
            ..Default::default()
        })
        .into()
}

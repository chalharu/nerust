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
                    key: KeyCode::new(format!("{:?}", event.physical_key)),
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

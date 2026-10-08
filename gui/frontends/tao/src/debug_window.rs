//! SPIKE-ONLY image prototype window (spike/debugger-ui-prototype-3).
//! Deleted with the spike branch. Minimal iced window showing the two
//! pattern-table images. Session actions run in the handle; the iced
//! program stays pure data (spike-1 wiring shape, re-derived).

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use iced::{Length, Size, Task, Theme, mouse, theme};
use iced_tiny_skia::{
    Renderer,
    graphics::compositor::Compositor as _,
    window::{Compositor, Surface, compositor},
};
use iced_winit::{
    Clipboard,
    graphics::Viewport,
    program::{self, Program},
    runtime::user_interface::{Cache, UserInterface},
};
use nerust_core_traits::debugger::{DebugImage, ImageFormat};
use nerust_gui_shell::session::SessionHandle;

#[cfg(target_os = "macos")]
use tao::platform::macos::WindowBuilderExtMacOS;
use tao::{
    event_loop::EventLoopWindowTarget,
    window::{Window as TaoWindow, WindowBuilder},
};

use crate::tao_conversions::{default_font, tao_modifiers_to_iced};

/// Mechanical blit: indexed pixels through the palette to RGBA.
/// No system knowledge: widths and palette come from the descriptor.
/// Direct RGB images pass pixels through with full alpha.
fn image_rgba(image: &DebugImage) -> Vec<u8> {
    match image.format {
        ImageFormat::Indexed { .. } => {
            let mut out = Vec::with_capacity(image.pixels.len() * 4);
            for &px in &image.pixels {
                let rgb = image.palette.get(px as usize).copied().unwrap_or([0, 0, 0]);
                out.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 0xFF]);
            }
            out
        }
        ImageFormat::Rgb8 => {
            let mut out = Vec::with_capacity(image.pixels.len() / 3 * 4);
            let (chunks, _) = image.pixels.as_chunks::<3>();
            for rgb in chunks {
                out.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 0xFF]);
            }
            out
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum DebugMessage {
    Refresh,
    SetData(DebugViewData),
}

#[derive(Debug, Clone, Default)]
pub(crate) struct DebugViewData {
    left: Option<iced::widget::image::Handle>,
    right: Option<iced::widget::image::Handle>,
    status: String,
    refresh_micros: u128,
}

pub(crate) struct DebugAppProgram {
    initial: DebugViewData,
}

impl Program for DebugAppProgram {
    type State = DebugViewData;
    type Message = DebugMessage;
    type Theme = Theme;
    type Renderer = iced_tiny_skia::Renderer;
    type Executor = iced_winit::futures::backend::default::Executor;

    fn name() -> &'static str {
        "nerust_spike_image"
    }

    fn settings(&self) -> iced::Settings {
        iced::Settings {
            default_font: default_font(),
            default_text_size: iced::Pixels(14.0),
            ..Default::default()
        }
    }

    fn window(&self) -> Option<iced::window::Settings> {
        None
    }

    fn boot(&self) -> (Self::State, Task<Self::Message>) {
        (self.initial.clone(), Task::none())
    }

    fn update(&self, state: &mut Self::State, message: Self::Message) -> Task<Self::Message> {
        if let DebugMessage::SetData(data) = message {
            *state = data;
        }
        Task::none()
    }

    fn view<'a>(
        &self,
        state: &'a Self::State,
        _window: iced::window::Id,
    ) -> iced::Element<'a, Self::Message, Self::Theme, Self::Renderer> {
        use iced::widget::{button, column, image, row, text};
        let left: iced::Element<'_, DebugMessage, Theme, Renderer> = match &state.left {
            Some(h) => image(h.clone()).width(256).height(256).into(),
            None => text("no left image").into(),
        };
        let right: iced::Element<'_, DebugMessage, Theme, Renderer> = match &state.right {
            Some(h) => image(h.clone()).width(256).height(256).into(),
            None => text("no right image").into(),
        };
        column![
            row![
                button("Refresh").on_press(DebugMessage::Refresh),
                text(state.status.as_str()).size(12),
            ]
            .spacing(12),
            row![left, right].spacing(12),
            text(format!("refresh={}µs", state.refresh_micros)).size(12),
        ]
        .spacing(8)
        .padding(12)
        .width(Length::Shrink)
        .into()
    }
}

struct DebugUiState {
    ui: std::mem::ManuallyDrop<
        UserInterface<'static, DebugMessage, iced::Theme, iced_tiny_skia::Renderer>,
    >,
    instance: program::Instance<DebugAppProgram>,
}

impl DebugUiState {
    fn build_ui(
        instance: &program::Instance<DebugAppProgram>,
        window_id: iced::window::Id,
        bounds: Size,
        cache: Cache,
        renderer: &mut iced_tiny_skia::Renderer,
    ) -> UserInterface<'static, DebugMessage, iced::Theme, iced_tiny_skia::Renderer> {
        unsafe {
            std::mem::transmute::<
                UserInterface<'_, DebugMessage, iced::Theme, iced_tiny_skia::Renderer>,
                UserInterface<'static, DebugMessage, iced::Theme, iced_tiny_skia::Renderer>,
            >(UserInterface::build(
                instance.view(window_id),
                bounds,
                cache,
                renderer,
            ))
        }
    }

    fn process_messages(
        &mut self,
        messages: Vec<DebugMessage>,
        window_id: iced::window::Id,
        bounds: Size,
        renderer: &mut iced_tiny_skia::Renderer,
    ) {
        if messages.is_empty() {
            return;
        }
        let placeholder = std::mem::replace(
            &mut *self.ui,
            Self::build_ui(
                &self.instance,
                window_id,
                bounds,
                Cache::default(),
                renderer,
            ),
        );
        let cache = placeholder.into_cache();
        for msg in messages {
            let _task = self.instance.update(msg);
        }
        let stale = std::mem::replace(
            &mut *self.ui,
            Self::build_ui(&self.instance, window_id, bounds, cache, renderer),
        );
        let _ = stale.into_cache();
    }

    fn ui_mut(
        &mut self,
    ) -> &mut UserInterface<'static, DebugMessage, iced::Theme, iced_tiny_skia::Renderer> {
        &mut self.ui
    }
}

impl Drop for DebugUiState {
    fn drop(&mut self) {
        unsafe { std::mem::ManuallyDrop::drop(&mut self.ui) };
    }
}

pub(crate) struct DebugWindowHandle {
    pub(crate) window: Arc<TaoWindow>,
    window_id: iced::window::Id,
    ui_state: DebugUiState,
    compositor: Compositor,
    surface: Surface,
    backend: Renderer,
    viewport_physical: (u32, u32),
    pub(crate) scale_factor: f32,
    pub(crate) modifiers: iced::keyboard::Modifiers,
    pub(crate) should_close: Arc<AtomicBool>,
    cursor: mouse::Cursor,
    clipboard: Clipboard,
}

impl DebugWindowHandle {
    pub(crate) fn new(
        event_loop: &EventLoopWindowTarget<crate::app_menu::UserEvent>,
    ) -> Option<Self> {
        let should_close = Arc::new(AtomicBool::new(false));
        #[cfg_attr(not(target_os = "macos"), expect(unused_mut))]
        let mut wb = WindowBuilder::new()
            .with_title("Pattern tables (spike)")
            .with_inner_size(tao::dpi::LogicalSize::new(600.0, 640.0));
        #[cfg(target_os = "macos")]
        {
            wb = wb.with_automatic_window_tabbing(false);
        }
        let window = Arc::new(match wb.build(event_loop) {
            Ok(w) => w,
            Err(e) => {
                log::error!("spike: failed to create debug window: {e}");
                return None;
            }
        });
        let window_id = iced::window::Id::unique();
        let program = DebugAppProgram {
            initial: DebugViewData {
                status: "press Refresh (pause the session first)".to_string(),
                ..Default::default()
            },
        };
        let (instance, _task) = program::Instance::new(program);
        let scale_factor = window.scale_factor() as f32;
        let window_size = window.inner_size();
        let viewport_physical = (window_size.width, window_size.height);
        let mut compositor = compositor::new(
            iced_tiny_skia::Settings {
                default_font: default_font(),
                default_text_size: iced::Pixels(14.0),
            },
            Arc::clone(&window),
        );
        let mut backend = compositor.create_renderer();
        let surface =
            compositor.create_surface(Arc::clone(&window), window_size.width, window_size.height);
        let bounds = Viewport::with_physical_size(
            Size::new(viewport_physical.0, viewport_physical.1),
            scale_factor,
        )
        .logical_size();
        let ui =
            DebugUiState::build_ui(&instance, window_id, bounds, Cache::default(), &mut backend);
        window.request_redraw();
        Some(Self {
            window,
            window_id,
            ui_state: DebugUiState {
                ui: std::mem::ManuallyDrop::new(ui),
                instance,
            },
            compositor,
            surface,
            backend,
            viewport_physical,
            scale_factor,
            modifiers: iced::keyboard::Modifiers::default(),
            should_close,
            cursor: mouse::Cursor::default(),
            clipboard: Clipboard::unconnected(),
        })
    }

    fn refresh(&mut self, session: &mut SessionHandle) -> DebugViewData {
        let t0 = Instant::now();
        let mut data = DebugViewData::default();
        match session.spike_images() {
            Err(e) => data.status = format!("session unavailable: {e:?}"),
            Ok(Err(e)) => data.status = format!("images refused: {e:?}"),
            Ok(Ok(images)) => {
                let mut iter = images.iter();
                if let Some(img) = iter.next() {
                    let rgba = image_rgba(img);
                    data.left = Some(iced::widget::image::Handle::from_rgba(
                        img.width, img.height, rgba,
                    ));
                }
                if let Some(img) = iter.next() {
                    let rgba = image_rgba(img);
                    data.right = Some(iced::widget::image::Handle::from_rgba(
                        img.width, img.height, rgba,
                    ));
                }
                data.status = format!("{} image(s)", images.len());
            }
        }
        data.refresh_micros = t0.elapsed().as_micros();
        data
    }

    fn handle_event(&mut self, mapped: iced::Event, session: &mut SessionHandle) {
        let mut messages = Vec::new();
        self.ui_state.ui_mut().update(
            &[mapped],
            self.cursor,
            &mut self.backend,
            &mut self.clipboard,
            &mut messages,
        );
        if messages.is_empty() {
            return;
        }
        let mut translated = Vec::with_capacity(messages.len());
        for msg in messages {
            match msg {
                DebugMessage::Refresh => {
                    translated.push(DebugMessage::SetData(self.refresh(session)));
                }
                DebugMessage::SetData(_) => {}
            }
        }
        let bounds = Viewport::with_physical_size(
            Size::new(self.viewport_physical.0, self.viewport_physical.1),
            self.scale_factor,
        )
        .logical_size();
        self.ui_state
            .process_messages(translated, self.window_id, bounds, &mut self.backend);
    }

    pub(crate) fn render(&mut self) {
        log::info!("spike: image render called");
        use iced::advanced::renderer;
        let theme = iced::Theme::Dark;
        let style = <iced::Theme as theme::Base>::base(&theme);
        let vp = Viewport::with_physical_size(
            Size::new(self.viewport_physical.0, self.viewport_physical.1),
            self.scale_factor,
        );
        let redraw_event = iced::Event::Window(iced::window::Event::RedrawRequested(
            std::time::Instant::now(),
        ));
        let _ = self.ui_state.ui_mut().update(
            &[redraw_event],
            self.cursor,
            &mut self.backend,
            &mut self.clipboard,
            &mut std::vec::Vec::new(),
        );
        self.ui_state.ui_mut().draw(
            &mut self.backend,
            &theme,
            &renderer::Style {
                text_color: style.text_color,
            },
            self.cursor,
        );
        if let Err(e) = self.compositor.present(
            &mut self.backend,
            &mut self.surface,
            &vp,
            iced::Color::BLACK,
            || {},
        ) {
            log::warn!("spike debug render present failed: {e:?}");
        }
    }

    /// SPIKE-ONLY instrumentation: refresh outside the event path.
    pub(crate) fn refresh_now(&mut self, session: &mut SessionHandle) {
        let data = self.refresh(session);
        log::info!(
            "spike: image refresh took {}µs: {}",
            data.refresh_micros,
            data.status
        );
        let bounds = Viewport::with_physical_size(
            Size::new(self.viewport_physical.0, self.viewport_physical.1),
            self.scale_factor,
        )
        .logical_size();
        self.ui_state.process_messages(
            vec![DebugMessage::SetData(data)],
            self.window_id,
            bounds,
            &mut self.backend,
        );
    }

    pub(crate) fn set_scale_factor(&mut self, sf: f32) {
        self.scale_factor = sf;
    }

    pub(crate) fn set_modifiers(&mut self, modifiers: tao::keyboard::ModifiersState) {
        self.modifiers = tao_modifiers_to_iced(modifiers);
    }

    pub(crate) fn update_modifiers_from_tao_event(&mut self, event: &tao::event::WindowEvent) {
        if let tao::event::WindowEvent::ModifiersChanged(state) = event {
            self.set_modifiers(*state);
        }
    }

    pub(crate) fn handle_tao_event(
        &mut self,
        event: tao::event::WindowEvent,
        session: &mut SessionHandle,
    ) {
        let mut cursor = std::mem::replace(&mut self.cursor, mouse::Cursor::Unavailable);
        let mut modifiers = self.modifiers;
        let should_close = self.should_close.clone();
        let scale_factor = self.scale_factor;
        let mapped = super::settings_window::convert_tao_window_event(
            event,
            &mut cursor,
            scale_factor,
            &mut modifiers,
            &should_close,
        );
        self.cursor = cursor;
        self.modifiers = modifiers;
        if let Some(iced_event) = mapped {
            self.handle_event(iced_event, session);
        }
    }

    pub(crate) fn resize(&mut self, width: u32, height: u32) {
        self.viewport_physical = (width, height);
        self.compositor
            .configure_surface(&mut self.surface, width, height);
    }

    pub(crate) fn should_close_now(&self) -> bool {
        self.should_close.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgba_blits_palette() {
        let image = DebugImage {
            id: "t",
            label_id: "t",
            width: 2,
            height: 1,
            format: ImageFormat::Indexed { bits_per_pixel: 2 },
            palette: vec![[1, 2, 3], [4, 5, 6], [7, 8, 9], [10, 11, 12]],
            pixels: vec![0, 3],
        };
        assert_eq!(image_rgba(&image), vec![1, 2, 3, 0xFF, 10, 11, 12, 0xFF]);
    }

    #[test]
    fn rgba_passes_direct_rgb_through() {
        let image = DebugImage {
            id: "t",
            label_id: "t",
            width: 2,
            height: 1,
            format: ImageFormat::Rgb8,
            palette: Vec::new(),
            pixels: vec![1, 2, 3, 4, 5, 6],
        };
        assert_eq!(image_rgba(&image), vec![1, 2, 3, 0xFF, 4, 5, 6, 0xFF]);
    }

    #[test]
    fn program_view_builds_headless() {
        let program = DebugAppProgram {
            initial: DebugViewData {
                status: "ok".to_string(),
                ..Default::default()
            },
        };
        let (state, _) = program.boot();
        let _element = program.view(&state, iced::window::Id::unique());
    }
}

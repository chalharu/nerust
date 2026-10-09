// SPIKE (iteration 10, DO NOT MERGE): PPU separate-window probe.
// Shares the main window's bridge read-only; renders pattern images
// at a fixed 2x integer scale (the spike-8 ~5.16x stretch verdict)
// plus panels and tile hover info (no$ VRAM-viewer style).
// Deleted with the branch.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use iced::{Point, Size, advanced::renderer, keyboard, mouse, theme};
use iced_tiny_skia::{
    Renderer,
    graphics::compositor::Compositor as _,
    window::{Compositor, Surface, compositor},
};
use iced_winit::{
    Clipboard,
    graphics::Viewport,
    program::{self, Program},
    runtime::{
        Task,
        user_interface::{Cache, UserInterface},
    },
};
use tao::{
    event_loop::EventLoopWindowTarget,
    window::{Window as TaoWindow, WindowBuilder},
};

use crate::{
    settings_window::convert_tao_window_event, spike_debug_window::SpikeDebugBridge,
    tao_conversions::default_font,
};

/// SPIKE (iteration 10): nearest-neighbor 2x upscale. The tiny-skia
/// image primitive draws handles at intrinsic size, so integer scale
/// is done on the pixels (presentation-only, tao-local). Production
/// work is an integer blit in the renderer.
fn spike_scale2x(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
    let (w, h) = (width as usize, height as usize);
    let mut out = vec![0u8; w * 2 * h * 2 * 4];
    for y in 0..h {
        for x in 0..w {
            let src = &rgba[(y * w + x) * 4..(y * w + x) * 4 + 4];
            for dy in 0..2 {
                for dx in 0..2 {
                    let dst = ((y * 2 + dy) * w * 2 + (x * 2 + dx)) * 4;
                    out[dst..dst + 4].copy_from_slice(src);
                }
            }
        }
    }
    out
}

/// Tile hover line: 2x display scale, 8x8 tiles, byte offset of the
/// tile start (+16 per tile). Generic geometry, no per-system branch.
fn spike_tile_hover(label: &str, width: u32, height: u32, point: Point) -> String {
    let sx = (point.x / 2.0).floor().clamp(0.0, width as f32 - 1.0) as u32;
    let sy = (point.y / 2.0).floor().clamp(0.0, height as f32 - 1.0) as u32;
    let cols = (width / 8).max(1);
    let idx = (sy / 8) * cols + (sx / 8);
    format!("{label} tile #{idx} (px {sx},{sy}, +{:04X})", idx * 16)
}

#[derive(Debug, Clone)]
pub(crate) enum SpikePpuMessage {
    Hovered(String),
    /// SPIKE (iteration 11): re-derive cached bytes from the bridge
    /// (one 2x scale per drain, never per hover).
    Sync,
}

pub(crate) struct SpikePpuState {
    bridge: Arc<SpikeDebugBridge>,
    /// SPIKE (iteration 11): scaled display bytes, derived on Sync
    /// only. `view` wraps handles; hover rebuilds never re-scale.
    #[allow(clippy::type_complexity)]
    scaled: Mutex<Vec<(String, u32, u32, Vec<u8>)>>,
    panels_cache: Mutex<String>,
    hover_cache: Mutex<String>,
}

pub(crate) struct SpikePpuProgram {
    pub(crate) bridge: Arc<SpikeDebugBridge>,
}

impl Program for SpikePpuProgram {
    type State = SpikePpuState;
    type Message = SpikePpuMessage;
    type Theme = iced::Theme;
    type Renderer = iced_tiny_skia::Renderer;
    type Executor = iced_winit::futures::backend::default::Executor;

    fn name() -> &'static str {
        "nerust_spike_ppu"
    }

    fn theme(&self, _state: &Self::State, _window: iced::window::Id) -> Option<Self::Theme> {
        Some(iced::Theme::Light)
    }

    fn settings(&self) -> iced::Settings {
        iced::Settings {
            default_font: default_font(),
            default_text_size: iced::Pixels(16.0),
            ..Default::default()
        }
    }

    fn window(&self) -> Option<iced::window::Settings> {
        None
    }

    fn boot(&self) -> (Self::State, Task<Self::Message>) {
        (
            SpikePpuState {
                bridge: Arc::clone(&self.bridge),
                scaled: Mutex::new(Vec::new()),
                panels_cache: Mutex::new(String::new()),
                hover_cache: Mutex::new(String::new()),
            },
            Task::none(),
        )
    }

    fn update(&self, state: &mut Self::State, message: Self::Message) -> Task<Self::Message> {
        let bridge = &state.bridge;
        match message {
            SpikePpuMessage::Hovered(line) => {
                *bridge.ppu_hover.lock().unwrap() = line.clone();
                *state.hover_cache.lock().unwrap() = line;
                bridge.ppu_invalidated.store(true, Ordering::Release);
            }
            SpikePpuMessage::Sync => {
                let images = bridge.images.lock().unwrap().clone();
                let mut scaled = Vec::with_capacity(images.len());
                for (label, width, height, rgba) in &images {
                    scaled.push((
                        label.clone(),
                        width * 2,
                        height * 2,
                        spike_scale2x(rgba, *width, *height),
                    ));
                }
                *state.scaled.lock().unwrap() = scaled;
                *state.panels_cache.lock().unwrap() = bridge.panels.lock().unwrap().clone();
            }
        }
        Task::none()
    }

    fn view<'a>(
        &self,
        state: &'a Self::State,
        _window: iced::window::Id,
    ) -> iced::Element<'a, Self::Message, Self::Theme, Self::Renderer> {
        use iced::Length;
        use iced::widget::{column, image, mouse_area, scrollable, text};
        // SPIKE (iteration 11): single coherent capture; scaled bytes
        // come from the Sync cache, never re-derived here.
        let scaled = state.scaled.lock().unwrap().clone();
        let panels = state.panels_cache.lock().unwrap().clone();
        let hover = state.hover_cache.lock().unwrap().clone();
        let mut content = column![
            text("SPIKE PPU probe (paused only)").size(18),
            text(if hover.is_empty() {
                "hover a tile".to_string()
            } else {
                hover
            })
            .size(14)
            .width(Length::Fill),
        ]
        .spacing(12)
        .padding(16)
        .width(Length::Fill);
        if scaled.is_empty() {
            content = content.push(text("(no images)").size(14));
        }
        for (label, width, height, rgba) in &scaled {
            content = content.push(text(label.clone()).size(14));
            let shown = image(image::Handle::from_rgba(*width, *height, rgba.clone()));
            let hover_label = label.clone();
            // Original (unscaled) geometry for the hover math: the
            // cache stores display size, so halve back here.
            let (hover_w, hover_h) = (width / 2, height / 2);
            content = content.push(mouse_area(shown).on_move(move |point| {
                SpikePpuMessage::Hovered(spike_tile_hover(&hover_label, hover_w, hover_h, point))
            }));
        }
        content = content.push(text("Panels").size(16));
        content = content.push(text(panels).size(14).font(iced::Font::MONOSPACE));
        // SPIKE (iteration 11): display-only content scrolls so Panels
        // stays reachable in the 620px window (no controls inside, so
        // the outside-controls rule is untouched).
        scrollable(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }
}

/// Owns Instance + Cache + UI, mirroring the main spike window.
pub(crate) struct SpikePpuUiState {
    ui: std::mem::ManuallyDrop<
        UserInterface<'static, SpikePpuMessage, iced::Theme, iced_tiny_skia::Renderer>,
    >,
    instance: program::Instance<SpikePpuProgram>,
    bridge: Arc<SpikeDebugBridge>,
}

impl SpikePpuUiState {
    fn build_ui(
        instance: &program::Instance<SpikePpuProgram>,
        window_id: iced::window::Id,
        bounds: Size,
        cache: Cache,
        renderer: &mut iced_tiny_skia::Renderer,
    ) -> UserInterface<'static, SpikePpuMessage, iced::Theme, iced_tiny_skia::Renderer> {
        unsafe {
            std::mem::transmute::<
                UserInterface<'_, SpikePpuMessage, iced::Theme, iced_tiny_skia::Renderer>,
                UserInterface<'static, SpikePpuMessage, iced::Theme, iced_tiny_skia::Renderer>,
            >(UserInterface::build(
                instance.view(window_id),
                bounds,
                cache,
                renderer,
            ))
        }
    }

    fn new(
        instance: program::Instance<SpikePpuProgram>,
        window_id: iced::window::Id,
        bounds: Size,
        renderer: &mut iced_tiny_skia::Renderer,
        bridge: Arc<SpikeDebugBridge>,
    ) -> Self {
        let ui = Self::build_ui(&instance, window_id, bounds, Cache::default(), renderer);
        Self {
            ui: std::mem::ManuallyDrop::new(ui),
            instance,
            bridge,
        }
    }

    fn ui_mut(
        &mut self,
    ) -> &mut UserInterface<'static, SpikePpuMessage, iced::Theme, iced_tiny_skia::Renderer> {
        &mut self.ui
    }

    fn process_messages(
        &mut self,
        messages: Vec<SpikePpuMessage>,
        window_id: iced::window::Id,
        bounds: Size,
        renderer: &mut iced_tiny_skia::Renderer,
    ) {
        if messages.is_empty() && !self.bridge.ppu_invalidated.load(Ordering::Acquire) {
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
        self.bridge.ppu_invalidated.store(false, Ordering::Release);
    }

    fn sync_from_bridge(
        &mut self,
        window_id: iced::window::Id,
        bounds: Size,
        renderer: &mut iced_tiny_skia::Renderer,
    ) {
        // SPIKE (iteration 11): derive cached bytes first (one scale
        // per drain), then rebuild the view from cache.
        let _task = self.instance.update(SpikePpuMessage::Sync);
        self.bridge.ppu_invalidated.store(true, Ordering::Release);
        self.process_messages(Vec::new(), window_id, bounds, renderer);
    }
}

impl Drop for SpikePpuUiState {
    fn drop(&mut self) {
        unsafe { std::mem::ManuallyDrop::drop(&mut self.ui) };
    }
}

pub(crate) struct SpikePpuWindowHandle {
    pub(crate) window: Arc<TaoWindow>,
    window_id: iced::window::Id,
    ui_state: SpikePpuUiState,
    renderer: SpikePpuRenderer,
    viewport_physical: (u32, u32),
    pub(crate) scale_factor: f32,
    pub(crate) modifiers: keyboard::Modifiers,
    pub(crate) should_close: Arc<AtomicBool>,
    cursor: mouse::Cursor,
    clipboard: Clipboard,
}

pub(crate) struct SpikePpuRenderer {
    compositor: Compositor,
    surface: Surface,
    backend: Renderer,
}

impl SpikePpuRenderer {
    fn present(
        &mut self,
        viewport: &Viewport,
        background_color: iced::Color,
    ) -> Result<(), iced_tiny_skia::graphics::compositor::SurfaceError> {
        self.compositor.present(
            &mut self.backend,
            &mut self.surface,
            viewport,
            background_color,
            || {},
        )
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.compositor
            .configure_surface(&mut self.surface, width, height);
    }
}

impl SpikePpuWindowHandle {
    pub(crate) fn new(
        bridge: Arc<SpikeDebugBridge>,
        event_loop: &EventLoopWindowTarget<crate::app_menu::UserEvent>,
    ) -> Option<Self> {
        let should_close = Arc::new(AtomicBool::new(false));
        let window = Arc::new(
            WindowBuilder::new()
                .with_title("PPU (spike)")
                .with_inner_size(tao::dpi::LogicalSize::new(560.0, 620.0))
                .build(event_loop)
                .map_err(|e| {
                    log::error!("failed to create spike PPU window: {e}");
                })
                .ok()?,
        );
        let window_id = iced::window::Id::unique();
        let program = SpikePpuProgram {
            bridge: Arc::clone(&bridge),
        };
        let (instance, _task) = program::Instance::new(program);
        let scale_factor = window.scale_factor() as f32;
        let window_size = window.inner_size();
        let viewport_physical = (window_size.width, window_size.height);
        let logical_size = window_size.to_logical::<f64>(scale_factor as f64);
        let bounds = Size::new(logical_size.width as f32, logical_size.height as f32);
        let mut compositor = compositor::new(
            iced_tiny_skia::Settings {
                default_font: default_font(),
                default_text_size: iced::Pixels(16.0),
            },
            Arc::clone(&window),
        );
        let mut renderer = compositor.create_renderer();
        let surface =
            compositor.create_surface(Arc::clone(&window), window_size.width, window_size.height);
        let ui_state = SpikePpuUiState::new(
            instance,
            window_id,
            bounds,
            &mut renderer,
            Arc::clone(&bridge),
        );
        window.request_redraw();
        Some(Self {
            window,
            window_id,
            ui_state,
            renderer: SpikePpuRenderer {
                compositor,
                surface,
                backend: renderer,
            },
            viewport_physical,
            scale_factor,
            modifiers: keyboard::Modifiers::default(),
            should_close,
            cursor: mouse::Cursor::default(),
            clipboard: Clipboard::unconnected(),
        })
    }

    pub(crate) fn sync_from_bridge(&mut self) {
        let bounds = Viewport::with_physical_size(
            Size::new(self.viewport_physical.0, self.viewport_physical.1),
            self.scale_factor,
        )
        .logical_size();
        self.ui_state
            .sync_from_bridge(self.window_id, bounds, &mut self.renderer.backend);
        self.window.request_redraw();
    }

    pub(crate) fn handle_event(&mut self, mapped: iced::Event) {
        let mut messages = Vec::new();
        self.ui_state.ui_mut().update(
            &[mapped],
            self.cursor,
            &mut self.renderer.backend,
            &mut self.clipboard,
            &mut messages,
        );
        if !messages.is_empty() {
            let bounds = Viewport::with_physical_size(
                Size::new(self.viewport_physical.0, self.viewport_physical.1),
                self.scale_factor,
            )
            .logical_size();
            self.ui_state.process_messages(
                messages,
                self.window_id,
                bounds,
                &mut self.renderer.backend,
            );
            self.window.request_redraw();
        }
    }

    pub(crate) fn render(&mut self) {
        let theme = iced::Theme::Light;
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
            &mut self.renderer.backend,
            &mut self.clipboard,
            &mut std::vec::Vec::new(),
        );
        self.ui_state.ui_mut().draw(
            &mut self.renderer.backend,
            &theme,
            &renderer::Style {
                text_color: style.text_color,
            },
            self.cursor,
        );
        if let Err(e) = self.renderer.present(&vp, style.background_color) {
            log::warn!("spike PPU render present failed: {e:?}");
        }
    }

    pub(crate) fn resize(&mut self, width: u32, height: u32) {
        self.viewport_physical = (width, height);
        self.renderer.resize(width, height);
    }

    pub(crate) fn set_scale_factor(&mut self, sf: f32) {
        self.scale_factor = sf;
    }

    pub(crate) fn set_modifiers(&mut self, modifiers: tao::keyboard::ModifiersState) {
        self.modifiers = crate::tao_conversions::tao_modifiers_to_iced(modifiers);
    }

    pub(crate) fn handle_tao_event(&mut self, event: tao::event::WindowEvent) {
        if let Some(iced_event) = convert_tao_window_event(
            event,
            &mut self.cursor,
            self.scale_factor,
            &mut self.modifiers,
            &self.should_close,
        ) {
            self.handle_event(iced_event);
        }
    }
}

//! PPU viewer window: system pattern images at a fixed 2x integer
//! scale plus panels and tile hover info.
//!
//! Display-only content scrolls so panels stay reachable in the fixed
//! window height. Scaled pictures derive once per drain (Sync); hover
//! rebuilds clone the cached handles, never the bytes.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use iced::{Length, Point, Size, Task, keyboard, mouse};
use iced_winit::{
    Clipboard,
    graphics::Viewport,
    program::{self, Program},
};

use tao::{
    event_loop::EventLoopWindowTarget, keyboard::ModifiersState as TaoModifiers,
    window::Window as TaoWindow,
};

use super::debug_window::DebugBridge;
use crate::{
    iced_shell::{ShellRenderer, UiShell, WindowScaffold, aux_settings, aux_theme, render_ui},
    tao_conversions::convert_tao_window_event,
};

/// Initial PPU viewer window size (verified Xvfb fit).
pub(crate) const PPU_WINDOW_SIZE: (f64, f64) = (560.0, 620.0);

use nerust_gui_viewmodel::debugger::scale2x_nearest;

/// Tile hover line: 2x display scale, 8x8 tiles, byte offset of the
/// tile start (+16 per tile). Generic geometry, no per-system branch.
fn tile_hover_text(
    language: nerust_gui_settings::language::AppLanguage,
    label: &str,
    width: u32,
    height: u32,
    point: Point,
) -> String {
    let sx = (point.x / 2.0).floor().clamp(0.0, width as f32 - 1.0) as u32;
    let sy = (point.y / 2.0).floor().clamp(0.0, height as f32 - 1.0) as u32;
    let cols = (width / 8).max(1);
    let idx = (sy / 8) * cols + (sx / 8);
    nerust_settings_core::i18n::tile_hover(language, label, idx, sx, sy, idx * 16)
}

#[derive(Debug, Clone)]
pub(crate) enum PpuMessage {
    Hovered(String),
    /// Re-derive cached pictures from the bridge (one 2x scale per
    /// drain, never per hover or view).
    Sync,
}

pub(crate) struct PpuState {
    bridge: Arc<DebugBridge>,
    /// Scaled display pictures `(label, w, h, handle)`, Sync-derived.
    /// Handles (cheap Arc clones) ride here instead of raw bytes so
    /// hover-driven rebuilds never re-copy megabytes per mouse move:
    /// the 2x scale runs once per drain, never per view.
    #[allow(clippy::type_complexity)]
    scaled: Mutex<Vec<(String, u32, u32, iced::widget::image::Handle)>>,
    panels_cache: Mutex<String>,
    hover_cache: Mutex<String>,
    /// Generation of the source images behind `scaled`. Sync
    /// re-derives only when the drain reports a new generation:
    /// drains while paused re-run Sync with identical memory. Starts
    /// impossibly high so the first Sync always derives (generation
    /// zero is a valid first fetch).
    source_gen: Mutex<u64>,
}

pub(crate) struct PpuProgram {
    pub(crate) bridge: Arc<DebugBridge>,
}

impl Program for PpuProgram {
    type State = PpuState;
    type Message = PpuMessage;
    type Theme = iced::Theme;
    type Renderer = iced_tiny_skia::Renderer;
    type Executor = iced_winit::futures::backend::default::Executor;

    fn name() -> &'static str {
        "nerust_ppu_viewer"
    }

    fn theme(&self, _state: &Self::State, _window: iced::window::Id) -> Option<Self::Theme> {
        aux_theme()
    }

    fn settings(&self) -> iced::Settings {
        aux_settings()
    }

    fn window(&self) -> Option<iced::window::Settings> {
        None
    }

    fn boot(&self) -> (Self::State, Task<Self::Message>) {
        (
            PpuState {
                bridge: Arc::clone(&self.bridge),
                scaled: Mutex::new(Vec::new()),
                panels_cache: Mutex::new(String::new()),
                hover_cache: Mutex::new(String::new()),
                source_gen: Mutex::new(u64::MAX),
            },
            Task::none(),
        )
    }

    fn update(&self, state: &mut Self::State, message: Self::Message) -> Task<Self::Message> {
        let bridge = &state.bridge;
        match message {
            PpuMessage::Hovered(line) => {
                *bridge.ppu_hover.lock().unwrap() = line.clone();
                *state.hover_cache.lock().unwrap() = line;
                bridge.ppu_invalidated.store(true, Ordering::Release);
            }
            PpuMessage::Sync => {
                let display = bridge.display.lock().unwrap();
                let generation = display.images_gen;
                let mut source_gen = state.source_gen.lock().unwrap();
                if *source_gen != generation {
                    let mut scaled = Vec::with_capacity(display.images.len());
                    for (label, width, height, rgba) in &display.images {
                        let bytes = scale2x_nearest(rgba, *width, *height);
                        scaled.push((
                            label.clone(),
                            width * 2,
                            height * 2,
                            iced::widget::image::Handle::from_rgba(width * 2, height * 2, bytes),
                        ));
                    }
                    *state.scaled.lock().unwrap() = scaled;
                    *source_gen = generation;
                }
                *state.panels_cache.lock().unwrap() =
                    nerust_gui_viewmodel::debugger::format_panels(&display.panels);
            }
        }
        Task::none()
    }

    fn view<'a>(
        &self,
        state: &'a Self::State,
        _window: iced::window::Id,
    ) -> iced::Element<'a, Self::Message, Self::Theme, Self::Renderer> {
        use iced::widget::{column, image, mouse_area, scrollable, text};
        use nerust_settings_core::i18n::{UiText, text as ui_text};
        // Single coherent capture; scaled bytes come from the Sync
        // cache, never re-derived here.
        let language = state.bridge.language;
        let scaled = state.scaled.lock().unwrap().clone();
        let panels = state.panels_cache.lock().unwrap().clone();
        let hover = state.hover_cache.lock().unwrap().clone();
        let mut content = column![
            text(ui_text(language, UiText::PpuViewer)).size(18),
            text(if hover.is_empty() {
                ui_text(language, UiText::HoverTile).to_string()
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
            content = content.push(text(ui_text(language, UiText::NoImages)).size(14));
        }
        for (label, width, height, handle) in &scaled {
            content = content.push(text(label.clone()).size(14));
            // Arc clone only: the bytes were scaled once at Sync time.
            let shown = image(handle.clone());
            let hover_label = label.clone();
            // Cache stores display size; halve back for hover math.
            let (hover_w, hover_h) = (width / 2, height / 2);
            content = content.push(mouse_area(shown).on_move(move |point| {
                PpuMessage::Hovered(tile_hover_text(
                    language,
                    &hover_label,
                    hover_w,
                    hover_h,
                    point,
                ))
            }));
        }
        // Systems without observation tables (GBA/GBC) show no
        // Panels section at all instead of a puzzling marker.
        if nerust_gui_viewmodel::debugger::panels_visible(&panels) {
            content = content.push(text(ui_text(language, UiText::Panels)).size(16));
            content = content.push(text(panels).size(14).font(iced::Font::MONOSPACE));
        }
        scrollable(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }
}

pub(crate) struct PpuWindowHandle {
    pub(crate) window: Arc<TaoWindow>,
    window_id: iced::window::Id,
    ui_state: UiShell<PpuProgram>,
    renderer: ShellRenderer,
    viewport_physical: (u32, u32),
    pub(crate) scale_factor: f32,
    pub(crate) modifiers: keyboard::Modifiers,
    pub(crate) should_close: Arc<AtomicBool>,
    cursor: mouse::Cursor,
    clipboard: Clipboard,
    bridge: Arc<DebugBridge>,
}

impl PpuWindowHandle {
    pub(crate) fn new(
        bridge: Arc<DebugBridge>,
        event_loop: &EventLoopWindowTarget<crate::app_menu::UserEvent>,
        position: Option<(i32, i32)>,
    ) -> Option<Self> {
        use nerust_settings_core::i18n::{UiText, text as ui_text};
        let language = bridge.language;
        let should_close = Arc::new(AtomicBool::new(false));
        let title = ui_text(language, UiText::PpuViewer).to_string();
        let scaffold = WindowScaffold::open(
            event_loop,
            &title,
            PPU_WINDOW_SIZE,
            None,
            position,
            "PPU viewer window",
        )?;
        let crate::iced_shell::WindowScaffold {
            window,
            window_id,
            scale_factor,
            viewport_physical,
            bounds,
            compositor,
            mut renderer,
            surface,
        } = scaffold;
        let program = PpuProgram {
            bridge: Arc::clone(&bridge),
        };
        let (instance, _task) = program::Instance::new(program);
        let ui_state = UiShell::new(instance, window_id, bounds, &mut renderer);
        window.request_redraw();
        // The viewer opens on demand from the debugger toolbar: take
        // focus so keyboard and wheel go straight to the tiles.
        window.set_focus();
        Some(Self {
            window,
            window_id,
            ui_state,
            renderer: ShellRenderer {
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
            bridge,
        })
    }

    pub(crate) fn handle_event(&mut self, mapped: iced::Event) {
        let messages = self.ui_state.pump(
            mapped,
            self.cursor,
            &mut self.clipboard,
            &mut self.renderer.backend,
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
                &self.bridge.ppu_invalidated,
            );
            self.window.request_redraw();
        }
    }

    pub(crate) fn render(&mut self) {
        render_ui(
            &mut self.ui_state,
            &mut self.renderer,
            self.viewport_physical,
            self.scale_factor,
            self.cursor,
            &mut self.clipboard,
            "PPU viewer",
        );
    }

    pub(crate) fn resize(&mut self, width: u32, height: u32) {
        self.viewport_physical = (width, height);
        self.renderer.resize(width, height);
        // Same relayout contract as the debugger: retained tiles
        // reflow against the fresh bounds, no view rebuild.
        let bounds = crate::iced_shell::logical_bounds((width, height), self.scale_factor);
        self.ui_state.relayout(bounds, &mut self.renderer.backend);
        self.window.request_redraw();
    }

    pub(crate) fn set_scale_factor(&mut self, sf: f32) {
        self.scale_factor = sf;
        let bounds = crate::iced_shell::logical_bounds(self.viewport_physical, self.scale_factor);
        self.ui_state.relayout(bounds, &mut self.renderer.backend);
        self.window.request_redraw();
    }

    pub(crate) fn set_modifiers(&mut self, modifiers: TaoModifiers) {
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

    pub(crate) fn sync_from_bridge(&mut self) {
        let bounds = Viewport::with_physical_size(
            Size::new(self.viewport_physical.0, self.viewport_physical.1),
            self.scale_factor,
        )
        .logical_size();
        // Derive cached bytes first (one scale per drain), then
        // rebuild the view from cache.
        self.ui_state.push_message(PpuMessage::Sync);
        self.bridge.ppu_invalidated.store(true, Ordering::Release);
        self.ui_state.process_messages(
            Vec::new(),
            self.window_id,
            bounds,
            &mut self.renderer.backend,
            &self.bridge.ppu_invalidated,
        );
        self.window.request_redraw();
        // Deterministic pixels per drain: render synchronously
        // instead of depending on the next window event, so the
        // viewer never sits on a stale or blank frame.
        self.render();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_hover_reports_index_and_offset() {
        use nerust_gui_settings::language::AppLanguage;
        let line = tile_hover_text(
            AppLanguage::English,
            "Pattern left",
            128,
            128,
            Point::new(88.0, 26.0),
        );
        assert_eq!(line, "Pattern left tile #21 (px 44,13, +0x0150)");
    }

    #[test]
    fn tile_hover_clamps_to_table() {
        use nerust_gui_settings::language::AppLanguage;
        let line = tile_hover_text(
            AppLanguage::Japanese,
            "Pattern right",
            128,
            128,
            Point::new(500.0, 500.0),
        );
        assert!(line.starts_with("Pattern right タイル #255 "));
    }

    fn ppu_bridge() -> Arc<DebugBridge> {
        use nerust_gui_settings::language::AppLanguage;
        use nerust_gui_viewmodel::debugger::DisplayCache;
        let display = DisplayCache {
            images: vec![("Tiles".to_string(), 2, 1, vec![0xFF; 8])],
            images_gen: 5,
            ..DisplayCache::default()
        };
        Arc::new(DebugBridge::new(
            display,
            Vec::new(),
            0,
            AppLanguage::English,
        ))
    }

    fn ppu_state(bridge: &Arc<DebugBridge>) -> PpuState {
        PpuState {
            bridge: Arc::clone(bridge),
            scaled: Mutex::new(Vec::new()),
            panels_cache: Mutex::new(String::new()),
            hover_cache: Mutex::new(String::new()),
            source_gen: Mutex::new(u64::MAX),
        }
    }

    #[test]
    fn sync_derives_once_per_generation() {
        let bridge = ppu_bridge();
        let program = PpuProgram {
            bridge: Arc::clone(&bridge),
        };
        let mut state = ppu_state(&bridge);
        let _ = program.update(&mut state, PpuMessage::Sync);
        // First sync derives: one 2x image plus the generation.
        assert_eq!(state.scaled.lock().unwrap().len(), 1);
        assert_eq!(*state.source_gen.lock().unwrap(), 5);
        assert_eq!(state.scaled.lock().unwrap()[0].1, 4);
        assert_eq!(state.scaled.lock().unwrap()[0].2, 2);
        // Identical generation re-runs Sync without re-deriving.
        let _ = program.update(&mut state, PpuMessage::Sync);
        assert_eq!(*state.source_gen.lock().unwrap(), 5);
        // Hover only records the line, never re-derives.
        let _ = program.update(
            &mut state,
            PpuMessage::Hovered("Tiles tile #0 (px 0,0, +0000)".to_string()),
        );
        assert_eq!(
            *state.hover_cache.lock().unwrap(),
            "Tiles tile #0 (px 0,0, +0000)"
        );
        assert!(bridge.ppu_invalidated.load(Ordering::Acquire));
    }

    #[test]
    fn view_renders_scaled_empty_and_hover() {
        let bridge = ppu_bridge();
        let program = PpuProgram {
            bridge: Arc::clone(&bridge),
        };
        // Empty cache shows the degenerate, never a blank window.
        let state = ppu_state(&bridge);
        let _empty = program.view(&state, iced::window::Id::unique());
        drop(_empty);
        // Synced cache renders labels plus images plus hover text.
        let mut synced = ppu_state(&bridge);
        let _ = program.update(&mut synced, PpuMessage::Sync);
        let _ = program.update(
            &mut synced,
            PpuMessage::Hovered("Tiles tile #1 (px 8,0, +0010)".to_string()),
        );
        let _full = program.view(&synced, iced::window::Id::unique());
        drop(_full);
    }
}

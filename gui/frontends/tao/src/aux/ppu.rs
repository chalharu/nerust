//! PPU viewer: fixed 2x system pattern images plus panels and
//! tile hover info. Display-only content scrolls; scaled pictures
//! derive once per drain, hover clones cached handles.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use iced::{Length, Point, Task};
use iced_winit::program::{self, Program};

use tao::event_loop::EventLoopWindowTarget;

use super::bridge::{DebugBridge, lock_recovered};
use crate::aux::shell::{WindowScaffold, aux_settings, aux_theme};

/// Initial PPU viewer window size (verified Xvfb fit).
pub(crate) const PPU_WINDOW_SIZE: (f64, f64) = (560.0, 620.0);

use nerust_gui_viewmodel::debugger::scale2x_nearest;

/// Tile hover line: 2x display scale, 8x8 tiles, byte offset of the
/// tile start (+16 per tile). Generic geometry, no per-system branch.
fn tile_hover_text(
    language: nerust_settings_core::model::language::AppLanguage,
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
    /// Generation of the source images behind `scaled`. Starts
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
                *lock_recovered(&bridge.ppu_hover) = line.clone();
                *lock_recovered(&state.hover_cache) = line;
                bridge.ppu_invalidated.store(true, Ordering::Release);
            }
            PpuMessage::Sync => {
                let display = lock_recovered(&bridge.display);
                let generation = display.images_gen;
                let mut source_gen = lock_recovered(&state.source_gen);
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
                    *lock_recovered(&state.scaled) = scaled;
                    *source_gen = generation;
                }
                *lock_recovered(&state.panels_cache) =
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
        let scaled = lock_recovered(&state.scaled).clone();
        let panels = lock_recovered(&state.panels_cache).clone();
        let hover = lock_recovered(&state.hover_cache).clone();
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
    pub(crate) shell: crate::aux::shell::AuxShell<PpuProgram>,
    bridge: Arc<DebugBridge>,
}

impl PpuWindowHandle {
    pub(crate) fn new(
        bridge: Arc<DebugBridge>,
        event_loop: &EventLoopWindowTarget<crate::event::UserEvent>,
        position: Option<(i32, i32)>,
    ) -> Option<Self> {
        use nerust_settings_core::i18n::{UiText, text as ui_text};
        let language = bridge.language;
        let title = ui_text(language, UiText::PpuViewer).to_string();
        let scaffold = WindowScaffold::open(
            event_loop,
            &title,
            PPU_WINDOW_SIZE,
            None,
            position,
            "PPU viewer window",
        )?;
        let program = PpuProgram {
            bridge: Arc::clone(&bridge),
        };
        let (instance, _task) = program::Instance::new(program);
        let shell = crate::aux::shell::AuxShell::assemble(
            scaffold,
            instance,
            Arc::new(AtomicBool::new(false)),
        );
        // The viewer opens on demand from the debugger toolbar: take
        // focus so keyboard and wheel go straight to the tiles.
        shell.window.set_focus();
        Some(Self { shell, bridge })
    }

    pub(crate) fn handle_event(&mut self, mapped: iced::Event) {
        let (messages, outcome) = self.shell.ui_state.pump(
            mapped,
            self.shell.cursor,
            &mut self.shell.clipboard,
            &mut self.shell.renderer.backend,
        );
        match outcome {
            crate::aux::shell::PumpOutcome::Rebuild => {
                self.bridge
                    .ppu_invalidated
                    .store(true, std::sync::atomic::Ordering::Release);
            }
            crate::aux::shell::PumpOutcome::Repaint => {
                self.shell.window.request_redraw();
            }
            crate::aux::shell::PumpOutcome::Idle => {}
        }
        if !messages.is_empty() || matches!(outcome, crate::aux::shell::PumpOutcome::Rebuild) {
            let bounds = self.shell.logical_bounds();
            self.shell.ui_state.process_messages(
                messages,
                self.shell.window_id,
                bounds,
                &mut self.shell.renderer.backend,
                &self.bridge.ppu_invalidated,
            );
            self.shell.window.request_redraw();
        }
    }

    pub(crate) fn render(&mut self) {
        self.shell.render("PPU viewer");
    }

    pub(crate) fn handle_tao_event(&mut self, event: tao::event::WindowEvent) {
        if let Some(iced_event) = self.shell.feed_tao_event(event) {
            self.handle_event(iced_event);
        }
    }

    pub(crate) fn sync_from_bridge(&mut self) {
        let bounds = self.shell.logical_bounds();
        // Derive cached bytes first (one scale per drain), then
        // rebuild the view from cache.
        self.shell.ui_state.push_message(PpuMessage::Sync);
        self.bridge.ppu_invalidated.store(true, Ordering::Release);
        self.shell.ui_state.process_messages(
            Vec::new(),
            self.shell.window_id,
            bounds,
            &mut self.shell.renderer.backend,
            &self.bridge.ppu_invalidated,
        );
        self.shell.window.request_redraw();
        // Deterministic pixels per drain: render synchronously
        // instead of depending on the next window event, so the
        // viewer never sits on a stale or blank frame.
        self.render();
    }
}

impl crate::aux::shell::AuxWindow for PpuWindowHandle {
    fn pump_window_event(&mut self, event: tao::event::WindowEvent) {
        self.shell.handle_common_event(&event);
        self.handle_tao_event(event);
        self.shell.window.request_redraw();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_hover_reports_index_and_offset() {
        use nerust_settings_core::model::language::AppLanguage;
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
        use nerust_settings_core::model::language::AppLanguage;
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
        use nerust_gui_viewmodel::debugger::DisplayCache;
        use nerust_settings_core::model::language::AppLanguage;
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
        assert_eq!(lock_recovered(&state.scaled).len(), 1);
        assert_eq!(*lock_recovered(&state.source_gen), 5);
        assert_eq!(lock_recovered(&state.scaled)[0].1, 4);
        assert_eq!(lock_recovered(&state.scaled)[0].2, 2);
        // Identical generation re-runs Sync without re-deriving.
        let _ = program.update(&mut state, PpuMessage::Sync);
        assert_eq!(*lock_recovered(&state.source_gen), 5);
        // Hover only records the line, never re-derives.
        let _ = program.update(
            &mut state,
            PpuMessage::Hovered("Tiles tile #0 (px 0,0, +0000)".to_string()),
        );
        assert_eq!(
            *lock_recovered(&state.hover_cache),
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

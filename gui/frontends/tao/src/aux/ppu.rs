//! PPU viewer: 2x system pattern images plus panels and tile hover info.
//! Scaled pictures derive once per drain; hover clones cached handles.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use iced::{Length, Point, Task};
use iced_winit::program::{self, Program};

use tao::event_loop::EventLoopWindowTarget;

use super::bridge::{DebugBridge, lock_recovered};
use crate::aux::shell::{AuxSurface, aux_settings, aux_theme};

/// Initial PPU viewer window size (verified Xvfb fit).
pub(crate) const PPU_WINDOW_SIZE: (f64, f64) = (560.0, 620.0);

use super::focus::{AuxRender as _, AuxShellRef};

use super::registry::AuxController;
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
    /// Hot pictures plus cold text, shared by `Arc`; Sync swaps the whole
    /// snapshot once per drain while hover clones only the `Arc`s.
    snapshot: Mutex<Arc<PpuSnapshot>>,
}

/// Hot pictures plus cold text, shared by `Arc` so hover-driven
/// rebuilds never re-copy megabytes per mouse move.
type ScaledPictures = Arc<[(String, u32, u32, iced::widget::image::Handle)]>;

struct PpuSnapshot {
    /// Scaled display pictures `(label, w, h, handle)`, Sync-derived.
    /// Shared `Arc`: hover updates clone the `Arc`, never the bytes.
    #[allow(clippy::type_complexity)]
    scaled: ScaledPictures,
    panels: Arc<str>,
    hover: Arc<str>,
    /// Generation of the source images behind `scaled`.
    /// Starts at `u64::MAX` so the first Sync always derives.
    source_gen: u64,
}

impl PpuSnapshot {
    fn empty() -> Arc<Self> {
        Arc::new(Self {
            scaled: Arc::from([]),
            panels: Arc::from(""),
            hover: Arc::from(""),
            source_gen: u64::MAX,
        })
    }
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
                snapshot: Mutex::new(PpuSnapshot::empty()),
            },
            Task::none(),
        )
    }

    fn update(&self, state: &mut Self::State, message: Self::Message) -> Task<Self::Message> {
        let bridge = &state.bridge;
        match message {
            PpuMessage::Hovered(line) => {
                *lock_recovered(&bridge.ppu_hover) = line.clone();
                let current = Arc::clone(&lock_recovered(&state.snapshot));
                // Hover swaps only the hover line; the scaled `Arc` is cloned.
                *lock_recovered(&state.snapshot) = Arc::new(PpuSnapshot {
                    scaled: Arc::clone(&current.scaled),
                    panels: Arc::clone(&current.panels),
                    hover: Arc::from(line),
                    source_gen: current.source_gen,
                });
                bridge.ppu_invalidated.store(true, Ordering::Release);
            }
            PpuMessage::Sync => {
                // Clone the source under the lock, then scale lock-free:
                // the 2x derivation must never hold the display guard.
                let (images, generation, panels) = {
                    let display = lock_recovered(&bridge.display);
                    (
                        display.images.clone(),
                        display.images_gen,
                        nerust_gui_viewmodel::debugger::format_panels(&display.panels),
                    )
                };
                let current = Arc::clone(&lock_recovered(&state.snapshot));
                // Sync swaps once per drain: scaled re-derived only on
                // generation change, hover preserved as-is.
                let scaled: ScaledPictures = if current.source_gen != generation {
                    let mut scaled = Vec::with_capacity(images.len());
                    for (label, width, height, rgba) in &images {
                        let bytes = scale2x_nearest(rgba, *width, *height);
                        scaled.push((
                            label.clone(),
                            width * 2,
                            height * 2,
                            iced::widget::image::Handle::from_rgba(width * 2, height * 2, bytes),
                        ));
                    }
                    Arc::from(scaled)
                } else {
                    Arc::clone(&current.scaled)
                };
                let panels: Arc<str> = Arc::from(panels);
                *lock_recovered(&state.snapshot) = Arc::new(PpuSnapshot {
                    scaled,
                    panels,
                    hover: Arc::clone(&current.hover),
                    source_gen: generation,
                });
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
        // Single coherent capture: one lock, one `Arc`; scaled bytes
        // come from the Sync cache, never re-derived here.
        let language = state.bridge.language;
        let snapshot = Arc::clone(&lock_recovered(&state.snapshot));
        // Borrow the scaled `Arc` (handles clone per row, cheap Arc);
        // cold text copies out as owned strings for the widgets.
        let scaled = &snapshot.scaled;
        let panels = snapshot.panels.to_string();
        let hover = snapshot.hover.to_string();
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
        for (label, width, height, handle) in scaled.iter() {
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
    shell: crate::aux::shell::AuxShell<PpuProgram>,
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
        let surface = AuxSurface::open(
            event_loop,
            &title,
            PPU_WINDOW_SIZE,
            None,
            position,
            "PPU viewer window",
            Arc::new(AtomicBool::new(false)),
        )
        .map_err(|error| {
            // Typed scaffold failure: surface the tagged native detail here.
            log::error!("{error}");
            error
        })
        .ok()?;
        let program = PpuProgram {
            bridge: Arc::clone(&bridge),
        };
        let (instance, _task) = program::Instance::new(program);
        let shell = crate::aux::shell::AuxShell::assemble(surface, instance);
        // The viewer opens on demand from the debugger toolbar: take
        // focus so keyboard and wheel go straight to the tiles.
        shell.focus_window();
        Some(Self { shell, bridge })
    }

    pub(crate) fn sync_from_bridge(&mut self) {
        // Derive cached bytes first, then rebuild the view from cache.
        self.bridge.ppu_invalidated.store(true, Ordering::Release);
        self.shell
            .process(vec![PpuMessage::Sync], &self.bridge.ppu_invalidated);
        // Render synchronously so the viewer never sits on a stale frame.
        self.render();
    }
}

impl AuxShellRef for PpuWindowHandle {
    fn kind(&self) -> super::focus::AuxKind {
        super::focus::AuxKind::Ppu
    }

    fn shell(&self) -> &dyn crate::aux::shell::AuxShellOps {
        &self.shell
    }

    fn shell_mut(&mut self) -> &mut dyn crate::aux::shell::AuxShellOps {
        &mut self.shell
    }
}

impl AuxController for PpuWindowHandle {
    fn sync_view(&mut self) {
        self.sync_from_bridge();
    }
}

impl super::focus::AuxEvents for PpuWindowHandle {
    fn handle(&mut self, mapped: iced::Event) -> bool {
        self.shell
            .pump_finish(mapped, None, &self.bridge.ppu_invalidated)
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
            snapshot: Mutex::new(PpuSnapshot::empty()),
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
        assert_eq!(lock_recovered(&state.snapshot).scaled.len(), 1);
        assert_eq!(lock_recovered(&state.snapshot).source_gen, 5);
        assert_eq!(lock_recovered(&state.snapshot).scaled[0].1, 4);
        assert_eq!(lock_recovered(&state.snapshot).scaled[0].2, 2);
        // Identical generation re-runs Sync without re-deriving.
        let _ = program.update(&mut state, PpuMessage::Sync);
        assert_eq!(lock_recovered(&state.snapshot).source_gen, 5);
        // Hover only records the line, never re-derives.
        let _ = program.update(
            &mut state,
            PpuMessage::Hovered("Tiles tile #0 (px 0,0, +0000)".to_string()),
        );
        assert_eq!(
            &*lock_recovered(&state.snapshot).hover,
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
    #[test]
    fn program_boot_and_chrome_are_headless() {
        let bridge = ppu_bridge();
        let program = PpuProgram {
            bridge: Arc::clone(&bridge),
        };
        assert_eq!(PpuProgram::name(), "nerust_ppu_viewer");
        assert!(program.window().is_none());
        let _ = program.settings();
        let (state, _task) = program.boot();
        let theme = program.theme(&state, iced::window::Id::unique());
        assert!(matches!(theme, Some(iced::Theme::Light)));
        let _ = program.view(&state, iced::window::Id::unique());
        drop(state);
    }
}

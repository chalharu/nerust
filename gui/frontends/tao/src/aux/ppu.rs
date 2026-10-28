//! PPU viewer: 2x system pattern images plus panels and tile hover info.
//! Scaled pictures derive once per drain; hover clones cached handles.

use std::sync::{
    Arc, RwLock,
    atomic::{AtomicBool, Ordering},
};

use iced::{Length, Task};
use iced_winit::program::{self, Program};

use tao::event_loop::EventLoopWindowTarget;

use super::bridge::DebugBridge;
use nerust_frontend_viewdefs::debugger::PpuChrome;

/// Poison-safe snapshot read: recovers the last snapshot after a panic.
fn snapshot_read(
    snapshot: &RwLock<Arc<PpuSnapshot>>,
) -> std::sync::RwLockReadGuard<'_, Arc<PpuSnapshot>> {
    snapshot.read().unwrap_or_else(|e| {
        log::warn!("ppu snapshot recovered after poisoning; state may be stale");
        e.into_inner()
    })
}

/// Poison-safe snapshot write: recovers the last snapshot after a panic.
fn snapshot_write(
    snapshot: &RwLock<Arc<PpuSnapshot>>,
) -> std::sync::RwLockWriteGuard<'_, Arc<PpuSnapshot>> {
    snapshot.write().unwrap_or_else(|e| {
        log::warn!("ppu snapshot recovered after poisoning; state may be stale");
        e.into_inner()
    })
}
use crate::aux::shell::{AuxSurface, aux_settings, aux_theme};

/// Initial PPU viewer window size (verified Xvfb fit).
pub(crate) const PPU_WINDOW_SIZE: (f64, f64) = (560.0, 620.0);

use super::focus::{AuxRender as _, AuxShellRef};

use super::registry::AuxController;
#[cfg(test)]
use nerust_gui_viewmodel::debugger::DebugImageRgba;

#[derive(Debug, Clone)]
pub(crate) enum PpuMessage {
    Hovered(String),
    /// Re-derive cached pictures from the bridge (one 2x scale per
    /// drain, never per hover or view).
    Sync,
}

pub(crate) struct PpuState {
    bridge: Arc<DebugBridge>,
    /// Hot pictures plus cold text, shared by `Arc`; `RwLock` for read-mostly sharing.
    snapshot: RwLock<Arc<PpuSnapshot>>,
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
    /// Pre-localized chrome from the host bridge; refreshed on Sync.
    chrome: PpuChrome,
    /// Generation of the source images behind `scaled`.
    /// Starts at `u64::MAX` so the first Sync always derives.
    source_gen: u64,
}

impl PpuSnapshot {
    fn empty(chrome: PpuChrome) -> Arc<Self> {
        Arc::new(Self {
            scaled: Arc::from([]),
            panels: Arc::from(""),
            hover: Arc::from(""),
            chrome,
            source_gen: u64::MAX,
        })
    }
}

pub(crate) struct PpuProgram {
    pub(crate) bridge: Arc<DebugBridge>,
}

impl super::window::GroupedProgram for PpuProgram {
    const KIND: super::focus::AuxKind = super::focus::AuxKind::Ppu;
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
                snapshot: RwLock::new(PpuSnapshot::empty(self.bridge.ppu_chrome())),
            },
            Task::none(),
        )
    }

    fn update(&self, state: &mut Self::State, message: Self::Message) -> Task<Self::Message> {
        let bridge = &state.bridge;
        match message {
            PpuMessage::Hovered(line) => {
                // Same-line hovers skip everything: no bridge write,
                // no snapshot alloc, no redraw.
                if snapshot_read(&state.snapshot).hover.as_ref() == line {
                    return Task::none();
                }
                bridge.set_hover(line.clone());
                let current = Arc::clone(&snapshot_read(&state.snapshot));
                // Hover swaps only the hover line; the scaled `Arc` is cloned.
                *snapshot_write(&state.snapshot) = Arc::new(PpuSnapshot {
                    scaled: Arc::clone(&current.scaled),
                    panels: Arc::clone(&current.panels),
                    hover: Arc::from(line),
                    chrome: current.chrome.clone(),
                    source_gen: current.source_gen,
                });
                bridge.ppu_invalidated.store(true, Ordering::Release);
            }
            PpuMessage::Sync => {
                // View-owned derivation: scaling and panel text live in
                // view-defs, so no formatter or image-type import is needed.
                let generation = bridge.images_generation();
                let panels = bridge.formatted_panels();
                let current = Arc::clone(&snapshot_read(&state.snapshot));
                // Sync swaps once per drain: scaled re-derived only on
                // generation change, hover preserved as-is.
                let scaled: ScaledPictures = if current.source_gen != generation {
                    let mut scaled = Vec::new();
                    for image in bridge.scaled_pictures() {
                        scaled.push((
                            image.label,
                            image.width,
                            image.height,
                            iced::widget::image::Handle::from_rgba(
                                image.width,
                                image.height,
                                image.rgba,
                            ),
                        ));
                    }
                    Arc::from(scaled)
                } else {
                    Arc::clone(&current.scaled)
                };
                let panels: Arc<str> = Arc::from(panels);
                *snapshot_write(&state.snapshot) = Arc::new(PpuSnapshot {
                    scaled,
                    panels,
                    hover: Arc::clone(&current.hover),
                    chrome: bridge.ppu_chrome(),
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
        // Single coherent capture: one lock, one `Arc`; scaled bytes
        // come from the Sync cache, never re-derived here. All chrome
        // strings arrive pre-localized in the snapshot.
        let snapshot = Arc::clone(&snapshot_read(&state.snapshot));
        // Borrow the scaled `Arc` (handles clone per row, cheap Arc). Cold text borrows first:
        // owned copies materialize only for shown sections, never for hidden or empty ones.
        let scaled = &snapshot.scaled;
        let hover_text: &Arc<str> = &snapshot.hover;
        let panels_text: &Arc<str> = &snapshot.panels;
        let chrome = &snapshot.chrome;
        let mut content = column![
            text(chrome.title.clone()).size(18),
            text(if hover_text.is_empty() {
                chrome.hover_empty.clone()
            } else {
                hover_text.to_string()
            })
            .size(14)
            .width(Length::Fill),
        ]
        .spacing(12)
        .padding(16)
        .width(Length::Fill);
        if scaled.is_empty() {
            content = content.push(text(chrome.no_images.clone()).size(14));
        }
        let state_bridge = Arc::clone(&state.bridge);
        for (label, width, height, handle) in scaled.iter() {
            content = content.push(text(label.clone()).size(14));
            // Arc clone only: the bytes were scaled once at Sync time.
            let shown = image(handle.clone());
            let hover_label = label.clone();
            let row_bridge = Arc::clone(&state_bridge);
            // Cache stores display size; halve back for hover math.
            let (hover_w, hover_h) = (width / 2, height / 2);
            content = content.push(mouse_area(shown).on_move(move |point| {
                PpuMessage::Hovered(row_bridge.hover_line(
                    &hover_label,
                    hover_w,
                    hover_h,
                    f64::from(point.x),
                    f64::from(point.y),
                ))
            }));
        }
        // Systems without observation tables (GBA/GBC) show no
        // Panels section at all instead of a puzzling marker.
        if state.bridge.has_panels() {
            content = content.push(text(chrome.panels_title.clone()).size(16));
            content = content.push(
                text(panels_text.to_string())
                    .size(14)
                    .font(iced::Font::MONOSPACE),
            );
        }
        scrollable(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }
}

pub(crate) struct PpuWindowHandle {
    shell: crate::aux::shell::AuxShell<super::window::WindowProgram<PpuProgram>>,
    bridge: Arc<DebugBridge>,
}

impl PpuWindowHandle {
    pub(crate) fn new(
        window: nerust_frontend_host::window::Window,
        bridge: Arc<DebugBridge>,
        event_loop: &EventLoopWindowTarget<crate::event::UserEvent>,
        position: Option<(i32, i32)>,
    ) -> Option<Self> {
        let surface = AuxSurface::open(
            event_loop,
            &window.title,
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
        let (instance, _task) =
            program::Instance::new(super::window::WindowProgram { window, program });
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
    fn outer_ppu_window_uses_common_title_and_existing_grouped_pipeline() {
        use super::super::window::WindowProgram;
        use nerust_frontend_host::{focus::AuxKind, window::Window};

        let mut window = Window::auxiliary(AuxKind::Ppu);
        window.title = "owned PPU title".into();
        let mut program = WindowProgram {
            window,
            program: PpuProgram {
                bridge: ppu_bridge(),
            },
        };
        let (mut state, _) = program.boot();
        let id = iced::window::Id::unique();
        assert_eq!(program.title(&state, id), "owned PPU title");
        drop(program.update(&mut state, PpuMessage::Sync));
        assert_eq!(snapshot_read(&state.snapshot).scaled.len(), 1);
        drop(program.view(&state, id));
        // A different grouped contents cannot accidentally render this program.
        program.window.contents = Window::auxiliary(AuxKind::Debug).contents;
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                drop(program.view(&state, id));
            }))
            .is_err()
        );
    }

    #[test]
    fn tile_hover_reports_index_and_offset() {
        let bridge = ppu_bridge();
        let line = bridge.hover_line("Pattern left", 128, 128, 88.0, 26.0);
        assert_eq!(line, "Pattern left tile #21 (px 44,13, +0x0150)");
    }

    #[test]
    fn tile_hover_clamps_to_table() {
        use nerust_settings_core::model::language::AppLanguage;
        let bridge = Arc::new(DebugBridge::new(
            nerust_gui_viewmodel::debugger::DisplayCache::default(),
            Vec::new(),
            0,
            AppLanguage::Japanese,
        ));
        let line = bridge.hover_line("Pattern right", 128, 128, 500.0, 500.0);
        assert!(line.starts_with("Pattern right タイル #255 "));
    }

    fn ppu_bridge() -> Arc<DebugBridge> {
        use nerust_gui_viewmodel::debugger::DisplayCache;
        use nerust_settings_core::model::language::AppLanguage;
        let display = DisplayCache {
            images: vec![DebugImageRgba {
                label: "Tiles".to_string(),
                width: 2,
                height: 1,
                rgba: vec![0xFF; 8],
            }],
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
            snapshot: RwLock::new(PpuSnapshot::empty(bridge.ppu_chrome())),
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
        assert_eq!(snapshot_read(&state.snapshot).scaled.len(), 1);
        assert_eq!(snapshot_read(&state.snapshot).source_gen, 5);
        assert_eq!(snapshot_read(&state.snapshot).scaled[0].1, 4);
        assert_eq!(snapshot_read(&state.snapshot).scaled[0].2, 2);
        let scaled = Arc::clone(&snapshot_read(&state.snapshot).scaled);
        // Identical generation re-runs Sync without re-deriving.
        let _ = program.update(&mut state, PpuMessage::Sync);
        assert_eq!(snapshot_read(&state.snapshot).source_gen, 5);
        assert!(Arc::ptr_eq(&scaled, &snapshot_read(&state.snapshot).scaled));
        // Hover only records the line, never re-derives.
        let _ = program.update(
            &mut state,
            PpuMessage::Hovered("Tiles tile #0 (px 0,0, +0000)".to_string()),
        );
        assert_eq!(
            &*snapshot_read(&state.snapshot).hover,
            "Tiles tile #0 (px 0,0, +0000)"
        );
        assert!(bridge.ppu_invalidated.load(Ordering::Acquire));
    }

    #[test]
    fn same_line_hover_skips_bridge_and_snapshot() {
        let bridge = ppu_bridge();
        let program = PpuProgram {
            bridge: Arc::clone(&bridge),
        };
        let mut state = ppu_state(&bridge);
        // First hover records and invalidates.
        let _ = program.update(&mut state, PpuMessage::Hovered("row".to_string()));
        bridge.ppu_invalidated.store(false, Ordering::Release);
        // Same line again: no bridge write, no snapshot swap, no redraw flag.
        let _ = program.update(&mut state, PpuMessage::Hovered("row".to_string()));
        assert!(!bridge.ppu_invalidated.load(Ordering::Acquire));
        assert_eq!(&*snapshot_read(&state.snapshot).hover, "row");
        assert_eq!(bridge.hover(), "row");
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

    #[test]
    fn sync_with_empty_images_keeps_empty_cache() {
        use nerust_gui_viewmodel::debugger::DisplayCache;
        use nerust_settings_core::model::language::AppLanguage;
        let bridge = Arc::new(DebugBridge::new(
            DisplayCache::default(),
            Vec::new(),
            0,
            AppLanguage::English,
        ));
        let program = PpuProgram {
            bridge: Arc::clone(&bridge),
        };
        let mut state = ppu_state(&bridge);
        // Generation zero differs from the empty sentinel: sync
        // records it without deriving any pictures.
        let _ = program.update(&mut state, PpuMessage::Sync);
        assert!(snapshot_read(&state.snapshot).scaled.is_empty());
        assert_eq!(snapshot_read(&state.snapshot).source_gen, 0);
    }

    #[test]
    fn sync_rederives_on_generation_change_and_keeps_hover() {
        let bridge = ppu_bridge();
        let program = PpuProgram {
            bridge: Arc::clone(&bridge),
        };
        let mut state = ppu_state(&bridge);
        let _ = program.update(&mut state, PpuMessage::Sync);
        let _ = program.update(&mut state, PpuMessage::Hovered("Tiles hover".to_string()));
        // New images plus new generation: scaled re-derived, hover kept.
        bridge.update_display(|display| {
            display.images = vec![DebugImageRgba {
                label: "Map".to_string(),
                width: 1,
                height: 1,
                rgba: vec![0x00; 4],
            }];
            display.images_gen = 6;
        });
        let _ = program.update(&mut state, PpuMessage::Sync);
        let snapshot = snapshot_read(&state.snapshot);
        assert_eq!(snapshot.scaled.len(), 1);
        assert_eq!(snapshot.scaled[0].0, "Map");
        assert_eq!(snapshot.source_gen, 6);
        assert_eq!(&*snapshot.hover, "Tiles hover");
    }

    #[test]
    fn view_shows_panels_section_only_when_present() {
        use nerust_core_traits::debugger::{ColumnKind, DebugPanel, PanelColumn};
        use nerust_gui_viewmodel::debugger::DisplayCache;
        use nerust_settings_core::model::language::AppLanguage;
        static COLUMNS: &[PanelColumn] = &[PanelColumn {
            id: "t",
            label_id: "T",
            kind: ColumnKind::Text,
        }];
        let bridge = Arc::new(DebugBridge::new(
            DisplayCache {
                panels: vec![DebugPanel {
                    id: "p",
                    label_id: "P",
                    columns: COLUMNS,
                    rows: Vec::new(),
                }],
                ..DisplayCache::default()
            },
            Vec::new(),
            0,
            AppLanguage::English,
        ));
        let program = PpuProgram {
            bridge: Arc::clone(&bridge),
        };
        let mut state = ppu_state(&bridge);
        let _ = program.update(&mut state, PpuMessage::Sync);
        // Non-empty panels render the section; empty ones skip it.
        let _full = program.view(&state, iced::window::Id::unique());
        drop(_full);
        assert!(nerust_gui_viewmodel::debugger::panels_visible(
            &snapshot_read(&state.snapshot).panels
        ));
    }
}

//! PPU viewer: 2x system pattern images plus panels and tile hover info.
//! Pictures and chrome arrive pushed from the app; hover coordinates queue
//! into the host drain for app-side formatting.

use std::{
    cell::RefCell,
    rc::Rc,
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, Ordering},
    },
};

use iced::{Length, Task};
use iced_winit::program::{self, Program};

use tao::event_loop::EventLoopWindowTarget;

use nerust_frontend_host::debugger::{PpuChrome, ScaledPicture};

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

/// Raw hover coordinates: the host drain formats them app-side, so the
/// view never names the language or tile geometry.
#[derive(Debug, Clone)]
pub(crate) struct HoverAt {
    pub(crate) label: String,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) x: f64,
    pub(crate) y: f64,
}

#[derive(Debug, Clone)]
pub(crate) enum PpuMessage {
    Hovered(String),
    /// Hover coordinates for the host drain (formats app-side).
    HoverRequest(HoverAt),
    /// Re-derive cached pictures from pushed data (one 2x scale per
    /// generation change, never per hover or view).
    Sync,
}

/// Hover queue: iced view pushes coordinates, the host drain consumes.
pub(crate) type PpuQueue = Rc<RefCell<Vec<HoverAt>>>;

pub(crate) struct PpuState {
    snapshot: RwLock<Arc<PpuSnapshot>>,
    queue: PpuQueue,
    pending: PendingPush,
}

/// Pushed payload waiting for Sync derivation. Shared with the handle so
/// the host turn can stage pictures without touching iced state.
pub(crate) type PendingPush = Rc<RefCell<Option<PpuPictures>>>;

/// Hot pictures plus cold text, shared by `Arc` so hover-driven
/// rebuilds never re-copy megabytes per mouse move.
type ScaledPictures = Arc<[(String, u32, u32, iced::widget::image::Handle)]>;

struct PpuSnapshot {
    /// Scaled display pictures `(label, w, h, handle)`, Sync-derived.
    /// Shared `Arc`: hover updates clone the `Arc`, never the bytes.
    #[allow(clippy::type_complexity)]
    scaled: ScaledPictures,
    panels: Arc<str>,
    has_panels: bool,
    hover: Arc<str>,
    /// Pre-localized chrome pushed from the app; refreshed on Sync.
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
            has_panels: false,
            hover: Arc::from(""),
            chrome,
            source_gen: u64::MAX,
        })
    }
}

/// Pushed picture payload: raw bytes plus generation, before 2x derivation.
#[derive(Debug, Clone, Default)]
pub(crate) struct PpuPictures {
    pub(crate) pictures: Vec<ScaledPicture>,
    pub(crate) panels: String,
    pub(crate) has_panels: bool,
    pub(crate) chrome: PpuChrome,
    pub(crate) generation: u64,
}

pub(crate) struct PpuProgram {
    pub(crate) queue: PpuQueue,
    pub(crate) pending: PendingPush,
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
                snapshot: RwLock::new(PpuSnapshot::empty(PpuChrome {
                    title: String::new(),
                    hover_empty: String::new(),
                    no_images: String::new(),
                    panels_title: String::new(),
                })),
                queue: Rc::clone(&self.queue),
                pending: Rc::clone(&self.pending),
            },
            Task::none(),
        )
    }

    fn update(&self, state: &mut Self::State, message: Self::Message) -> Task<Self::Message> {
        match message {
            PpuMessage::Hovered(line) => {
                // Same-line hovers skip everything: no snapshot alloc, no redraw.
                if snapshot_read(&state.snapshot).hover.as_ref() == line {
                    return Task::none();
                }
                let current = Arc::clone(&snapshot_read(&state.snapshot));
                // Hover swaps only the hover line; the scaled `Arc` is cloned.
                *snapshot_write(&state.snapshot) = Arc::new(PpuSnapshot {
                    scaled: Arc::clone(&current.scaled),
                    panels: Arc::clone(&current.panels),
                    has_panels: current.has_panels,
                    hover: Arc::from(line),
                    chrome: current.chrome.clone(),
                    source_gen: current.source_gen,
                });
            }
            PpuMessage::HoverRequest(at) => {
                state.queue.borrow_mut().push(at);
            }
            PpuMessage::Sync => {
                // Derivation runs on pushed data staged by the handle;
                // Sync only swaps once per pushed payload. Pictures arrive
                // already 2x-scaled from the app; handles wrap the bytes.
                let current = Arc::clone(&snapshot_read(&state.snapshot));
                if let Some(pushed) = state.pending.borrow_mut().take() {
                    let scaled: ScaledPictures = if current.source_gen != pushed.generation {
                        let mut scaled = Vec::new();
                        for image in &pushed.pictures {
                            scaled.push((
                                image.label.clone(),
                                image.width,
                                image.height,
                                iced::widget::image::Handle::from_rgba(
                                    image.width,
                                    image.height,
                                    image.rgba.clone(),
                                ),
                            ));
                        }
                        Arc::from(scaled)
                    } else {
                        Arc::clone(&current.scaled)
                    };
                    let panels: Arc<str> = Arc::from(pushed.panels);
                    *snapshot_write(&state.snapshot) = Arc::new(PpuSnapshot {
                        scaled,
                        panels,
                        has_panels: pushed.has_panels,
                        hover: Arc::clone(&current.hover),
                        chrome: pushed.chrome,
                        source_gen: pushed.generation,
                    });
                }
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
        for (label, width, height, handle) in scaled.iter() {
            content = content.push(text(label.clone()).size(14));
            // Arc clone only: the bytes were scaled once at Sync time.
            let shown = image(handle.clone());
            let hover_label = label.clone();
            // Cache stores display size; halve back for hover math.
            let (hover_w, hover_h) = (width / 2, height / 2);
            content = content.push(mouse_area(shown).on_move(move |point| {
                PpuMessage::HoverRequest(HoverAt {
                    label: hover_label.clone(),
                    width: hover_w,
                    height: hover_h,
                    x: f64::from(point.x),
                    y: f64::from(point.y),
                })
            }));
        }
        // Systems without observation tables (GBA/GBC) show no
        // Panels section instead of a puzzling marker.
        if snapshot.has_panels {
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
    queue: PpuQueue,
    pending: PendingPush,
    invalidated: Arc<AtomicBool>,
}

impl PpuWindowHandle {
    pub(crate) fn new(
        window: nerust_frontend_host::window::Window,
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
        let queue = Rc::new(RefCell::new(Vec::new()));
        let pending = Rc::new(RefCell::new(None));
        let program = PpuProgram {
            queue: Rc::clone(&queue),
            pending: Rc::clone(&pending),
        };
        let (instance, _task) =
            program::Instance::new(super::window::WindowProgram { window, program });
        let shell = crate::aux::shell::AuxShell::assemble(surface, instance);
        // The viewer opens on demand from the debugger toolbar: take
        // focus so keyboard and wheel go straight to the tiles.
        shell.focus_window();
        Some(Self {
            shell,
            queue,
            pending,
            invalidated: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Take queued hover coordinates for the host drain.
    pub(crate) fn drain_hover(&mut self) -> Vec<HoverAt> {
        std::mem::take(&mut *self.queue.borrow_mut())
    }

    /// Stage pushed pictures and derive handles on the iced thread.
    pub(crate) fn push_pictures(&mut self, pictures: PpuPictures) {
        *self.pending.borrow_mut() = Some(pictures);
        self.invalidated.store(true, Ordering::Release);
        self.shell
            .process(vec![PpuMessage::Sync], &self.invalidated);
        // Render synchronously so the viewer never sits on a stale frame.
        let _ = self.render();
    }

    /// Show app-formatted hover text without touching geometry here.
    pub(crate) fn push_hover(&mut self, line: String) {
        self.invalidated.store(true, Ordering::Release);
        self.shell
            .process(vec![PpuMessage::Hovered(line)], &self.invalidated);
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
    fn as_ppu(&mut self) -> Option<&mut PpuWindowHandle> {
        Some(self)
    }
}

impl super::focus::AuxEvents for PpuWindowHandle {
    fn handle(&mut self, mapped: iced::Event) -> bool {
        self.shell.pump_finish(mapped, None, &self.invalidated)
    }
}

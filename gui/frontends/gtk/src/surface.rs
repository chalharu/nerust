use std::{cell::RefCell, rc::Rc};

use gtk::{glib, prelude::*};
use nerust_frontend_host::{
    frame::{FrameCache, FrameCtx},
    loop_driver::{LoopHost, TurnLiveness, WakeNeed, drive_turn},
};
use nerust_render_traits::{SurfaceSize, renderer::PipelineFactory};

use super::{State, renderer::GtkRenderer};

pub(crate) struct SurfaceCore {
    window: gtk::ApplicationWindow,
    /// Game viewport composited by GTK under the in-window menubar. The
    /// backends render into memory; nothing ever presents a fullscreen
    /// native surface here, so the chrome stays visible.
    picture: gtk::Picture,
    renderer: Rc<RefCell<GtkRenderer>>,
    pub(crate) state: Rc<RefCell<State>>,
    pub(crate) osd: RefCell<super::osd::GtkOsd>,
    frames: RefCell<FrameCache>,
    retry: RefCell<nerust_frontend_kernel::retry::RenderRetry>,
}

pub(crate) type Surface = Rc<RefCell<SurfaceCore>>;

/// Main-window ports only. GTK debugger flights live in separate native
/// windows (using the shared DrainQueue), so this turn's debug port is a no-op.
/// Native focus callbacks and window chrome timers retain their own cadence.
struct GtkTurn<'a> {
    surface: &'a Surface,
    core: &'a SurfaceCore,
    state: &'a mut State,
    physical_size: SurfaceSize,
}

impl LoopHost for GtkTurn<'_> {
    type Frame = FrameCtx;

    fn before_turn(&mut self) {
        // GTK focus/pause ownership is settled by native signal callbacks.
    }

    fn frame_ctx(&mut self) -> FrameCtx {
        self.core.frames.borrow_mut().frame_ctx(&self.state.session)
    }

    fn chrome(&mut self, _ctx: &FrameCtx) {
        // Existing native window callbacks own title/menu synchronization.
    }

    fn pump_debug(&mut self) -> WakeNeed {
        WakeNeed::default()
    }

    fn pump_osd(&mut self, ctx: &FrameCtx) -> WakeNeed {
        self.core.osd.borrow_mut().poll(
            self.surface,
            self.state,
            &self.core.renderer,
            &self.core.window,
            self.physical_size,
            ctx,
        )
    }

    fn pump_aux(&mut self) -> WakeNeed {
        let mut retry = self.core.retry.borrow_mut();
        if retry.take_due(std::time::Instant::now()) {
            self.core.picture.queue_draw();
        }
        WakeNeed {
            osd_deadline: retry.deadline(),
            ..WakeNeed::default()
        }
    }

    fn late_liveness(&mut self) -> TurnLiveness {
        let tick = self.state.session.tick_metrics();
        TurnLiveness {
            loaded: tick.loaded,
            paused: tick.paused,
            main_active: self.core.window.is_active(),
            // Follow belongs to the separate debugger/PPU native window.
            live_follow: false,
        }
    }

    fn request_redraw(&mut self) {
        self.core.picture.queue_draw();
    }
}

pub(crate) trait SurfaceExtend {
    fn bind(
        window: &gtk::ApplicationWindow,
        picture: &gtk::Picture,
        state: Rc<RefCell<State>>,
        factory: Rc<dyn PipelineFactory>,
    ) -> Surface;
    fn tick(&self) -> bool;
}

impl SurfaceExtend for Surface {
    fn bind(
        window: &gtk::ApplicationWindow,
        picture: &gtk::Picture,
        state: Rc<RefCell<State>>,
        factory: Rc<dyn PipelineFactory>,
    ) -> Surface {
        state.borrow_mut().renderer_reload_pending = true;
        let renderer = Rc::new(RefCell::new(GtkRenderer::new(factory)));
        let osd = super::osd::GtkOsd::new();
        let result = Rc::new(RefCell::new(SurfaceCore {
            window: window.clone(),
            picture: picture.clone(),
            renderer,
            state,
            osd: RefCell::new(osd),
            frames: RefCell::new(FrameCache::new()),
            retry: RefCell::new(Default::default()),
        }));
        {
            let result = result.clone();
            let _ = window.add_tick_callback(move |_window, _frame_clock| {
                glib::ControlFlow::from(result.tick())
            });
        }
        result
    }

    fn tick(&self) -> bool {
        let s = self.borrow();
        // Game area is the picture below the menubar (layout handles the
        // chrome, so no strip math here). Clamp negatives: hidden windows
        // report -1, and a negative `as u32` cast would explode the size.
        let allocation = s.picture.allocation();
        let width = allocation.width().max(0) as u32;
        let height = allocation.height().max(0) as u32;
        if width == 0 || height == 0 {
            return true;
        }
        let scale = s.window.scale_factor().max(1) as u32;
        let physical_size =
            SurfaceSize::new(width.saturating_mul(scale), height.saturating_mul(scale));

        // Contended borrows skip loudly: the next tick retries (backoff),
        // never a silent dropped frame.
        let swap = match s.state.try_borrow_mut() {
            Ok(mut state) => {
                state.swap_frame_buffer();
                state.render_profile().cloned().map(|profile| {
                    (
                        profile,
                        state.renderer_reload_pending || !s.renderer.borrow().attached(),
                    )
                })
            }
            Err(_) => {
                log::warn!("gtk tick: state busy, skipping frame");
                return true;
            }
        };
        let now = std::time::Instant::now();
        SurfaceCore::ensure_if_due(&s, swap, physical_size, now);
        SurfaceCore::poll_osd(self, &s, physical_size);
        SurfaceCore::render_image_if_ready(&s, physical_size);
        true
    }
}

impl SurfaceCore {
    /// Build (or rebuild) the pipeline when the profile moved or setup
    /// never completed. No present ever happens here: frames leave through
    /// [`Self::render_image_if_ready`] for the GTK picture to composite,
    /// so the in-window menubar stays visible. Attach runs outside the
    /// state borrow; outcome handling re-borrows briefly.
    fn ensure_if_due(
        s: &SurfaceCore,
        swap: Option<(nerust_render_traits::VideoRenderProfile, bool)>,
        physical_size: SurfaceSize,
        now: std::time::Instant,
    ) {
        let ready = s.retry.borrow().ready(now);
        let Some((profile, rebuild)) = swap else {
            return;
        };
        if !(ready && (rebuild || !s.renderer.borrow().attached())) {
            return;
        }
        let result = s.window.surface().and_then(|surf| {
            let display = gdk::Display::default()?;
            super::gdk_raw::with_raw_handles(&surf, &display, |wh, dh| {
                s.renderer
                    .borrow_mut()
                    .ensure_ready(wh, dh, &profile, physical_size)
            })
        });
        let mut state = match s.state.try_borrow_mut() {
            Ok(state) => state,
            Err(_) => {
                log::warn!("gtk tick: state busy after attach, skipping frame");
                return;
            }
        };
        match result {
            Some(Ok(())) => {
                state.renderer_reload_pending = false;
            }
            failure => {
                if s.retry.borrow_mut().failed(now) {
                    let detail = match failure {
                        Some(Err(error)) => error.to_string(),
                        _ => "native surface/display handles unavailable".into(),
                    };
                    state.session.report_external_error(
                        nerust_gui_shell::session::notification::FailureOrigin::Host,
                        format!("GTK renderer attach failed: {detail}"),
                    );
                }
            }
        }
    }

    /// Run the shared turn with a short borrow, outside the GPU pass. Prototype
    /// limitation: deadlines are observed but not translated to GLib timers;
    /// the existing GTK frame clock remains the drawing/polling driver.
    fn poll_osd(surface: &Surface, s: &SurfaceCore, physical_size: SurfaceSize) {
        let mut state = match s.state.try_borrow_mut() {
            Ok(state) => state,
            Err(_) => {
                log::warn!("gtk tick: state busy before poll, skipping frame");
                return;
            }
        };
        let outcome = drive_turn(&mut GtkTurn {
            surface,
            core: s,
            state: &mut state,
            physical_size,
        });
        // Redraw was delivered through the port. A future GLib wake adapter
        // can consume this deadline without changing the shared turn policy.
        if let Some(deadline) = outcome.deadline {
            log::trace!("gtk turn: frame clock owns wakeup; deadline {deadline:?}");
        }
    }

    /// Render one frame into memory and hand it to the GTK picture, which
    /// composites it under the menubar. Nothing ever presents a fullscreen
    /// native surface, so the chrome survives by construction.
    fn render_image_if_ready(s: &SurfaceCore, physical_size: SurfaceSize) {
        let state = match s.state.try_borrow_mut() {
            Ok(state) => state,
            Err(_) => {
                log::warn!("gtk tick: state busy before render, skipping frame");
                return;
            }
        };
        let Some(fb) = state.frame_buffer() else {
            return;
        };
        let Some(image) = s.renderer.borrow_mut().render_image(fb, physical_size) else {
            return;
        };
        let texture = gtk::gdk::MemoryTexture::new(
            image.width as i32,
            image.height as i32,
            gtk::gdk::MemoryFormat::R8g8b8a8,
            &gtk::glib::Bytes::from(&image.pixels),
            image.stride_bytes,
        );
        s.picture.set_paintable(Some(&texture));
        s.retry.borrow_mut().succeeded();
    }
}

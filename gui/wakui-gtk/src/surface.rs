use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Instant,
};

use gtk::{glib, prelude::*};
use wakui::loop_driver::{
    LoopHost, TurnLiveness, TurnOutcome, WakeNeed, drive_turn, flight_backstop,
};
use wakui::osd::ModalPresentation;
use wakui::viewport::ViewportRegion;

use super::State;
use wakui::debugger::DebuggerScope;

pub(crate) struct SurfaceCore<K: Copy + Eq + std::hash::Hash + 'static> {
    // The window owns the native frame callback, which owns Surface. Never
    // close that cycle with a strong reference back to the window.
    window: glib::WeakRef<gtk::ApplicationWindow>,
    /// Game viewport composited by GTK under the in-window menubar. The
    /// backends render into memory; nothing ever presents a fullscreen
    /// native surface here, so the chrome stays visible.
    picture: gtk::Picture,
    pub(crate) state: Rc<RefCell<State<K>>>,
    output_generation: Cell<Option<u64>>,
    retry: RefCell<wakui::retry::RenderRetry>,
    wake: RefCell<WakeAt>,
}

pub(crate) type Surface<K> = Rc<RefCell<SurfaceCore<K>>>;

/// Exactly one native one-shot. Only a Weak surface lives in its callback.
#[derive(Default)]
struct WakeAt {
    source: Option<glib::SourceId>,
    deadline: Option<Instant>,
}

impl WakeAt {
    fn cancel(&mut self) {
        self.deadline = None;
        if let Some(source) = self.source.take() {
            source.remove();
        }
    }

    fn schedule<K: Copy + Eq + std::hash::Hash + 'static>(
        &mut self,
        surface: &Surface<K>,
        deadline: Option<Instant>,
    ) {
        let Some(deadline) = deadline else {
            self.cancel();
            return;
        };
        if self.deadline.is_some_and(|old| old <= deadline) {
            return; // Repeated frame turns must not postpone an earlier source.
        }
        self.cancel();
        self.deadline = Some(deadline);
        let weak = Rc::downgrade(surface);
        self.source = Some(glib::timeout_add_local_once(
            deadline.saturating_duration_since(Instant::now()),
            move || {
                let Some(surface) = weak.upgrade() else {
                    return;
                };
                {
                    let s = surface.borrow();
                    let mut wake = s.wake.borrow_mut();
                    // GLib removes the firing source. Forget its id before the
                    // pump can cancel/rearm (or synchronously reenter GTK).
                    wake.source.take();
                    wake.deadline = None;
                }
                SurfaceCore::turn(&surface, false);
            },
        ));
    }
}

impl Drop for WakeAt {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// Main-window ports plus the generic debugger-widget drain (no kinds).
/// The 250ms aux timer owns native pushes and live ticks; this frame pump
/// keeps the view-side session fresh and folds poll wakes into the deadline.
struct GtkTurn<'a, K: Copy + Eq + std::hash::Hash + 'static> {
    core: &'a SurfaceCore<K>,
    window: &'a gtk::ApplicationWindow,
    state: &'a mut State<K>,
    physical_width: u32,
    physical_height: u32,
    client_height: u32,
    modal: Option<ModalPresentation>,
}

impl<K: Copy + Eq + std::hash::Hash + 'static> LoopHost for GtkTurn<'_, K> {
    type Frame = ();

    fn before_turn(&mut self) {
        // GTK focus/pause ownership is settled by native signal callbacks.
    }

    fn frame_ctx(&mut self) {}

    fn chrome(&mut self, _ctx: &Self::Frame) {
        // Existing native window callbacks own title/menu synchronization.
    }

    fn pump_debug(&mut self) -> WakeNeed {
        let id = match self.state.host.debug_widget() {
            Some(id) => id,
            None => return WakeNeed::default(),
        };
        self.state.host.widget_refresh(id);
        let handle = match self.state.host.debug_handle_for(id) {
            Some(handle) => handle,
            None => return WakeNeed::default(),
        };
        let _ = self.state.host.debug_reset_if_replaced(handle);
        let wake = self.state.host.debug_poll(handle);
        if self.state.host.chrome_drain_due() {
            self.state.host.debug_refresh(handle, DebuggerScope::All);
            self.state.host.widget_refresh(id);
        }
        wake
    }

    fn pump_osd(&mut self, _ctx: &Self::Frame) -> WakeNeed {
        let now = Instant::now();
        let turn = self.state.host.pump_osd(now);
        self.modal = turn.modal;
        self.state.host.sync_osd_viewport(
            self.physical_width,
            self.physical_height,
            self.client_height,
        );
        turn.wake
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
        TurnLiveness {
            loaded: self.state.host.tick_loaded(),
            paused: self.state.host.tick_paused(),
            main_active: self.window.is_active(),
            // Follow belongs to the separate debugger/PPU native window.
            live_follow: false,
        }
    }

    fn request_redraw(&mut self) {
        self.core.picture.queue_draw();
    }
}

pub(crate) trait SurfaceExtend<K: Copy + Eq + std::hash::Hash + 'static> {
    fn bind(
        window: &gtk::ApplicationWindow,
        picture: &gtk::Picture,
        state: Rc<RefCell<State<K>>>,
    ) -> Surface<K>;
    fn tick(&self) -> bool;
}

impl<K: Copy + Eq + std::hash::Hash + 'static> SurfaceExtend<K> for Surface<K> {
    fn bind(
        window: &gtk::ApplicationWindow,
        picture: &gtk::Picture,
        state: Rc<RefCell<State<K>>>,
    ) -> Surface<K> {
        state.borrow_mut().renderer_reload_pending = true;
        let result = Rc::new(RefCell::new(SurfaceCore {
            window: window.downgrade(),
            picture: picture.clone(),
            state,
            output_generation: Cell::new(None),
            retry: RefCell::new(Default::default()),
            wake: RefCell::new(WakeAt::default()),
        }));
        {
            // bind's caller discards the returned Surface: the native frame
            // callback owns it until window destruction, as before. Only the
            // auxiliary wake source is Weak; it must never extend this lifetime.
            let result = result.clone();
            let _ = window.add_tick_callback(move |_window, _frame_clock| {
                glib::ControlFlow::from(result.tick())
            });
        }
        result
    }

    fn tick(&self) -> bool {
        SurfaceCore::turn(self, true)
    }
}

impl<K: Copy + Eq + std::hash::Hash + 'static> SurfaceCore<K> {
    /// Deadlines pump even without a mapped allocation; only the native frame
    /// clock swaps/attaches/renders. Present effects after all borrows end.
    fn turn(surface: &Surface<K>, render_frame: bool) -> bool {
        let s = surface.borrow();
        let Some(window) = s.window.upgrade() else {
            s.wake.borrow_mut().cancel();
            return false;
        };
        if super::surface_state::surface_dead(&window) {
            s.wake.borrow_mut().cancel();
            return false;
        }
        // Game area is the picture below the menubar (layout handles the
        // chrome, so no strip math here). Clamp negatives: hidden windows
        // report -1, and a negative `as u32` cast would explode the size.
        let allocation = s.picture.allocation();
        let (width, height) = if s.picture.is_mapped() {
            (
                allocation.width().max(0) as u32,
                allocation.height().max(0) as u32,
            )
        } else {
            (0, 0)
        };
        let scale = window.scale_factor().max(1) as u32;
        let (physical_width, physical_height) =
            (width.saturating_mul(scale), height.saturating_mul(scale));
        let render_frame = render_frame && width != 0 && height != 0;

        // Hidden windows may have no next frame-clock tick: preserve a bounded
        // retry on contention instead of dropping the expired wake.
        match s.state.try_borrow_mut() {
            Ok(mut state) => {
                // Commit an armed auto-pause once per frame, mirroring the
                // tao burst-end commit. Arrival cancels via focus events.
                state.tick_focus_sync();
                if render_frame {
                    state.swap_frame_buffer();
                }
            }
            Err(_) => {
                log::warn!("gtk turn: state busy, retrying within 30ms");
                s.wake
                    .borrow_mut()
                    .schedule(surface, Some(flight_backstop(Instant::now())));
                return true;
            }
        };
        if render_frame {
            Self::ensure_if_due(&s, &window, physical_width, physical_height, Instant::now());
        }
        let effects = Self::poll_osd(&s, &window, physical_width, physical_height);
        let deadline = match &effects {
            Some((outcome, _)) => outcome.deadline,
            None => Some(flight_backstop(Instant::now())),
        };
        s.wake.borrow_mut().schedule(surface, deadline);
        drop(s);
        if let Some((_, modal)) = effects
            && let Some(presentation) = modal
        {
            crate::native::dialog::present_modal(surface, &window, presentation);
        }
        if render_frame {
            Self::render_image_if_ready(&surface.borrow(), physical_width, physical_height);
        }
        true
    }

    /// Build (or rebuild) the pipeline when the profile moved or setup
    /// never completed. No present ever happens here: frames leave through
    /// [`Self::render_image_if_ready`] for the GTK picture to composite,
    /// so the in-window menubar stays visible. Attach runs outside the
    /// state borrow; outcome handling re-borrows briefly.
    fn ensure_if_due(
        s: &SurfaceCore<K>,
        window: &gtk::ApplicationWindow,
        physical_width: u32,
        physical_height: u32,
        now: std::time::Instant,
    ) {
        if !s.retry.borrow().ready(now) {
            return;
        }
        let force = match s.state.try_borrow() {
            Ok(state) => state.renderer_reload_pending,
            Err(_) => return,
        };
        let result = window.surface().and_then(|surf| {
            let display = gdk::Display::default()?;
            super::gdk_raw::with_raw_handles(&surf, &display, |wh, dh| {
                let region = ViewportRegion {
                    window: wh,
                    display: dh,
                    width: physical_width,
                    height: physical_height,
                };
                match s.state.try_borrow_mut() {
                    Ok(mut state) => state.host.viewport_sync(region, force),
                    Err(_) => Ok(false),
                }
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
            Some(Ok(true)) => {
                state.renderer_reload_pending = false;
            }
            Some(Ok(false)) => {}
            failure => {
                if s.retry.borrow_mut().failed(now) {
                    let detail = match failure {
                        Some(Err(error)) => error,
                        _ => "native surface/display handles unavailable".into(),
                    };
                    state
                        .host
                        .report_host_error(format!("GTK renderer attach failed: {detail}"));
                }
            }
        }
    }

    /// Return owned effects; no native dialog may be presented in this scope.
    fn poll_osd(
        s: &SurfaceCore<K>,
        window: &gtk::ApplicationWindow,
        physical_width: u32,
        physical_height: u32,
    ) -> Option<(TurnOutcome, Option<ModalPresentation>)> {
        let generation = match s.state.try_borrow() {
            Ok(state) => state.host.viewport_generation(),
            Err(_) => {
                log::warn!("gtk tick: state busy before poll, skipping frame");
                return None;
            }
        };
        let client_height =
            (window.height().max(0) as u32).saturating_mul(window.scale_factor().max(1) as u32);
        let mut state = match s.state.try_borrow_mut() {
            Ok(state) => state,
            Err(_) => {
                log::warn!("gtk tick: state busy before poll, skipping frame");
                return None;
            }
        };
        if s.output_generation.replace(Some(generation)) != Some(generation) {
            state.host.invalidate_osd();
        }
        let mut turn = GtkTurn {
            core: s,
            window,
            state: &mut state,
            physical_width,
            physical_height,
            client_height,
            modal: None,
        };
        let outcome = drive_turn(&mut turn);
        Some((outcome, turn.modal))
    }

    /// Render one frame into memory and hand it to the GTK picture, which
    /// composites it under the menubar. Nothing ever presents a fullscreen
    /// native surface, so the chrome survives by construction.
    fn render_image_if_ready(s: &SurfaceCore<K>, physical_width: u32, physical_height: u32) {
        let mut state = match s.state.try_borrow_mut() {
            Ok(state) => state,
            Err(_) => {
                log::warn!("gtk tick: state busy before render, skipping frame");
                return;
            }
        };
        let Some(image) = state.host.viewport_capture(physical_width, physical_height) else {
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

#[cfg(test)]
mod wake_tests {
    use super::*;
    use std::{cell::Cell, time::Duration};

    struct Harness {
        window: gtk::ApplicationWindow,
        state: Rc<RefCell<State<crate::testkit::TestAux>>>,
        surface: Surface<crate::testkit::TestAux>,
    }

    impl Harness {
        fn new() -> Option<Self> {
            if !crate::test_support::ui_test_available() {
                eprintln!("SKIP: native wake test needs a display (run isolated under Xvfb)");
                return None;
            }
            // This tests native widgets/context, not GApplication membership.
            let window = gtk::ApplicationWindow::builder().build();
            let picture = gtk::Picture::new();
            window.set_child(Some(&picture));
            let state = crate::window::tests::model_state();
            let surface = Surface::bind(&window, &picture, state.clone());
            Some(Self {
                window,
                state,
                surface,
            })
        }

        fn schedule(&self, deadline: Option<Instant>) {
            self.surface
                .borrow()
                .wake
                .borrow_mut()
                .schedule(&self.surface, deadline);
        }

        fn source(&self) -> glib::Source {
            let s = self.surface.borrow();
            let wake = s.wake.borrow();
            glib::MainContext::default()
                .find_source_by_id(wake.source.as_ref().unwrap())
                .expect("pending source registered in the native context")
        }
    }

    impl Drop for Harness {
        fn drop(&mut self) {
            self.window.destroy();
        }
    }

    /// Native context dispatch, not a fake clock or a fixed polling actor.
    fn dispatch_until(mut done: impl FnMut() -> bool) {
        let limit = Instant::now() + Duration::from_secs(3);
        let context = glib::MainContext::default();
        while !done() {
            assert!(Instant::now() < limit, "native source did not dispatch");
            context.iteration(false);
            std::thread::yield_now();
        }
    }

    #[test]
    fn native_modal_can_ack_synchronously_during_map_without_retaining_surface() {
        use wakui::pause::PauseOwner;
        let Some(h) = Harness::new() else { return };
        let mapped = Rc::new(Cell::new(0));
        let dialogs = Rc::new(RefCell::new(Vec::<gtk::MessageDialog>::new()));
        let toplevels = gtk::Window::toplevels();
        let signal = toplevels.connect_items_changed({
            let weak = Rc::downgrade(&h.surface);
            let mapped = mapped.clone();
            let dialogs = dialogs.clone();
            move |list, position, _, added| {
                for i in position..position + added {
                    let Some(dialog) = list.item(i).and_downcast::<gtk::MessageDialog>() else {
                        continue;
                    };
                    dialog.connect_map({
                        let weak = weak.clone();
                        let mapped = mapped.clone();
                        move |dialog| {
                            let surface = weak.upgrade().unwrap();
                            assert!(surface.try_borrow_mut().is_ok());
                            let state = surface.borrow().state.clone();
                            assert!(state.try_borrow_mut().is_ok());
                            assert!(state.borrow().host.owns_pause(PauseOwner::Modal));
                            mapped.set(mapped.get() + 1);
                            dialog.response(gtk::ResponseType::Ok);
                            assert!(!state.borrow().host.owns_pause(PauseOwner::Modal));
                        }
                    });
                    dialogs.borrow_mut().push(dialog);
                }
            }
        });
        h.state
            .borrow_mut()
            .host
            .report_host_error("sync ACK".into());
        assert!(h.surface.tick());
        assert_eq!(mapped.get(), 1);
        assert!(!h.state.borrow().host.owns_pause(PauseOwner::Modal));
        toplevels.disconnect(signal);
        let weak = Rc::downgrade(&h.surface);
        drop(h);
        assert!(
            weak.upgrade().is_none(),
            "native dialog callbacks must not own Surface"
        );
        // A late duplicate response after teardown is harmless.
        dialogs.borrow()[0].response(gtk::ResponseType::Ok);
    }

    #[test]
    fn native_deadline_keeps_earlier_replaces_and_cancels() {
        let Some(h) = Harness::new() else { return };
        let now = Instant::now();
        let earlier = now + Duration::from_secs(10);
        h.schedule(Some(earlier));
        let first = h.source();
        h.schedule(Some(earlier));
        h.schedule(Some(now + Duration::from_secs(20)));
        assert_eq!(h.source(), first);
        assert_eq!(h.surface.borrow().wake.borrow().deadline, Some(earlier));
        let soonest = now + Duration::from_secs(5);
        h.schedule(Some(soonest));
        assert!(first.is_destroyed());
        let replacement = h.source();
        assert_ne!(replacement, first);
        assert_eq!(h.surface.borrow().wake.borrow().deadline, Some(soonest));
        h.schedule(None);
        assert!(replacement.is_destroyed());
        let s = h.surface.borrow();
        let wake = s.wake.borrow();
        assert!(wake.source.is_none());
        assert!(wake.deadline.is_none());
    }

    #[test]
    fn native_deadline_fires_and_pumps_zero_size_hidden_surface() {
        let Some(h) = Harness::new() else { return };
        assert!(!h.window.is_realized());
        assert!(!h.surface.borrow().picture.is_mapped());
        // An already-due auxiliary contribution proves the shared turn ran.
        h.surface
            .borrow()
            .retry
            .borrow_mut()
            .failed(Instant::now() - Duration::from_secs(1));
        h.schedule(Some(Instant::now()));
        let source = h.source();
        dispatch_until(|| source.is_destroyed());
        let s = h.surface.borrow();
        assert!(s.retry.borrow().deadline().is_none());
        assert!(s.wake.borrow().source.is_none());
        assert!(s.wake.borrow().deadline.is_none());
        assert!(!s.state.borrow().host.viewport_attached());
        assert_eq!(s.state.borrow().host.viewport_generation(), 0);
        assert!(s.picture.paintable().is_none());
        drop(s);
        // Frame-clock entry also pumps instead of returning early on zero size.
        h.surface
            .borrow()
            .retry
            .borrow_mut()
            .failed(Instant::now() - Duration::from_secs(1));
        assert!(h.surface.tick());
        assert!(h.surface.borrow().retry.borrow().deadline().is_none());
    }

    #[test]
    fn native_busy_state_rearms_within_30ms_and_recovers_while_hidden() {
        let Some(h) = Harness::new() else { return };
        h.surface
            .borrow()
            .retry
            .borrow_mut()
            .failed(Instant::now() - Duration::from_secs(1));
        h.schedule(Some(Instant::now()));
        let first = h.source();
        let state = h.state.borrow_mut();
        let before = Instant::now();
        dispatch_until(|| first.is_destroyed());
        let deadline = h.surface.borrow().wake.borrow().deadline.unwrap();
        assert!(deadline >= before + Duration::from_millis(30));
        assert!(deadline <= Instant::now() + Duration::from_millis(30));
        assert!(h.surface.borrow().retry.borrow().deadline().is_some());
        let retry = h.source();
        assert_ne!(retry, first);
        drop(state);
        dispatch_until(|| retry.is_destroyed());
        assert!(h.surface.borrow().wake.borrow().source.is_none());
        assert!(h.surface.borrow().retry.borrow().deadline().is_none());
    }

    #[test]
    fn native_drop_releases_weak_callback_and_removes_source() {
        let Some(h) = Harness::new() else { return };
        h.schedule(Some(Instant::now() + Duration::from_secs(60)));
        let source = h.source();
        let weak = Rc::downgrade(&h.surface);
        let window = h.window.clone();
        // Destroying the window releases the native frame callback's ownership;
        // the pending timeout must not keep Surface alive after that.
        drop(h);
        drop(window);
        assert!(weak.upgrade().is_none());
        assert!(source.is_destroyed());
    }

    #[test]
    fn native_deadline_pumps_after_realized_window_is_hidden() {
        let Some(h) = Harness::new() else { return };
        h.window.set_default_size(320, 240);
        h.window.present();
        dispatch_until(|| h.surface.borrow().picture.width() > 0);
        h.window.hide();
        assert!(h.window.is_realized());
        assert!(!h.surface.borrow().picture.is_mapped());
        h.schedule(Some(Instant::now()));
        let source = h.source();
        h.surface
            .borrow()
            .retry
            .borrow_mut()
            .failed(Instant::now() - Duration::from_secs(1));
        dispatch_until(|| source.is_destroyed());
        assert!(h.surface.borrow().retry.borrow().deadline().is_none());
        assert!(h.surface.borrow().wake.borrow().source.is_none());
        assert!(!h.surface.borrow().state.borrow().host.viewport_attached());
        assert!(h.surface.borrow().picture.paintable().is_none());
    }

    #[test]
    fn native_modal_map_can_borrow_state_surface_and_renderer_then_ack() {
        use wakui::pause::PauseOwner;

        let Some(h) = Harness::new() else { return };
        h.state
            .borrow_mut()
            .host
            .set_pause_owner(PauseOwner::Focus, true);
        let mapped = Rc::new(Cell::new(0));
        let dialogs = Rc::new(RefCell::new(Vec::<gtk::MessageDialog>::new()));
        let toplevels = gtk::Window::toplevels();
        let signal = toplevels.connect_items_changed({
            let weak = Rc::downgrade(&h.surface);
            let state = h.state.clone();
            let mapped = mapped.clone();
            let dialogs = dialogs.clone();
            move |list, position, _, added| {
                for i in position..position + added {
                    let Some(dialog) = list.item(i).and_downcast::<gtk::MessageDialog>() else {
                        continue;
                    };
                    dialog.connect_map({
                        let weak = weak.clone();
                        let state = state.clone();
                        let mapped = mapped.clone();
                        move |_| {
                            let surface = weak.upgrade().unwrap();
                            let s = surface.borrow_mut();
                            let state = state.borrow_mut();
                            assert!(state.host.owns_pause(PauseOwner::Modal));
                            // The firing source was cleared before entering GTK.
                            assert!(s.wake.borrow().source.is_none());
                            mapped.set(mapped.get() + 1);
                        }
                    });
                    dialogs.borrow_mut().push(dialog);
                }
            }
        });
        h.state.borrow_mut().host.report_host_error("first".into());
        h.schedule(Some(Instant::now()));
        dispatch_until(|| mapped.get() == 1);
        h.state.borrow_mut().host.report_host_error("second".into());
        assert!(h.surface.tick());
        assert_eq!(mapped.get(), 1, "only one dialog until ACK");
        let first = dialogs.borrow()[0].clone();
        first.response(gtk::ResponseType::Ok);
        assert_eq!(
            mapped.get(),
            2,
            "ACK presents the next batch without turn borrows"
        );
        assert!(h.state.borrow().host.owns_pause(PauseOwner::Modal));
        first.response(gtk::ResponseType::Ok);
        assert_eq!(
            mapped.get(),
            2,
            "stale response cannot advance the active dialog"
        );
        assert!(h.state.borrow().host.owns_pause(PauseOwner::Modal));
        let second = dialogs.borrow()[1].clone();
        second.response(gtk::ResponseType::Ok);
        assert!(!h.state.borrow().host.owns_pause(PauseOwner::Modal));
        second.response(gtk::ResponseType::Ok);
        assert!(!h.state.borrow().host.owns_pause(PauseOwner::Modal));
        assert!(h.state.borrow().host.owns_pause(PauseOwner::Focus));
        toplevels.disconnect(signal);
    }
}

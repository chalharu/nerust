use std::{cell::RefCell, rc::Rc, time::Instant};

use gtk::{glib, prelude::*};
use nerust_frontend_host::{
    frame::{FrameCache, FrameCtx},
    loop_driver::{LoopHost, TurnLiveness, TurnOutcome, WakeNeed, drive_turn, flight_backstop},
};
use nerust_render_traits::{SurfaceSize, renderer::PipelineFactory};

use super::{State, renderer::GtkRenderer};

pub(crate) struct SurfaceCore {
    // The window owns the native frame callback, which owns Surface. Never
    // close that cycle with a strong reference back to the window.
    window: glib::WeakRef<gtk::ApplicationWindow>,
    /// Game viewport composited by GTK under the in-window menubar. The
    /// backends render into memory; nothing ever presents a fullscreen
    /// native surface here, so the chrome stays visible.
    picture: gtk::Picture,
    renderer: Rc<RefCell<GtkRenderer>>,
    pub(crate) state: Rc<RefCell<State>>,
    pub(crate) osd: RefCell<super::osd::GtkOsd>,
    frames: RefCell<FrameCache>,
    retry: RefCell<nerust_frontend_kernel::retry::RenderRetry>,
    wake: RefCell<WakeAt>,
}

pub(crate) type Surface = Rc<RefCell<SurfaceCore>>;

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

    fn schedule(&mut self, surface: &Surface, deadline: Option<Instant>) {
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

/// Main-window ports only. GTK debugger flights live in separate native
/// windows (using the shared DrainQueue), so this turn's debug port is a no-op.
/// Native focus callbacks and window chrome timers retain their own cadence;
/// the debugger's separate 250ms timer is not unified with this wake adapter.
struct GtkTurn<'a> {
    core: &'a SurfaceCore,
    window: &'a gtk::ApplicationWindow,
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
            self.state,
            &self.core.renderer,
            self.window,
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
            main_active: self.window.is_active(),
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
            window: window.downgrade(),
            picture: picture.clone(),
            renderer,
            state,
            osd: RefCell::new(osd),
            frames: RefCell::new(FrameCache::new()),
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

impl SurfaceCore {
    /// Deadlines pump even without a mapped allocation; only the native frame
    /// clock swaps/attaches/renders. Present effects after all borrows end.
    fn turn(surface: &Surface, render_frame: bool) -> bool {
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
        let physical_size =
            SurfaceSize::new(width.saturating_mul(scale), height.saturating_mul(scale));
        let render_frame = render_frame && width != 0 && height != 0;

        // Hidden windows may have no next frame-clock tick: preserve a bounded
        // retry on contention instead of dropping the expired wake.
        let swap = match s.state.try_borrow_mut() {
            Ok(mut state) => {
                if render_frame {
                    state.swap_frame_buffer();
                    state.render_profile().cloned().map(|profile| {
                        (
                            profile,
                            state.renderer_reload_pending || !s.renderer.borrow().attached(),
                        )
                    })
                } else {
                    None
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
            Self::ensure_if_due(&s, &window, swap, physical_size, Instant::now());
        }
        let effects = Self::poll_osd(&s, &window, physical_size);
        let deadline = match &effects {
            Some((outcome, _)) => outcome.deadline,
            None => Some(flight_backstop(Instant::now())),
        };
        s.wake.borrow_mut().schedule(surface, deadline);
        if render_frame {
            Self::render_image_if_ready(&s, physical_size);
        }
        drop(s);
        if let Some((_, Some(detail))) = effects {
            super::osd::GtkOsd::present_modal(surface, &window, detail);
        }
        true
    }

    /// Build (or rebuild) the pipeline when the profile moved or setup
    /// never completed. No present ever happens here: frames leave through
    /// [`Self::render_image_if_ready`] for the GTK picture to composite,
    /// so the in-window menubar stays visible. Attach runs outside the
    /// state borrow; outcome handling re-borrows briefly.
    fn ensure_if_due(
        s: &SurfaceCore,
        window: &gtk::ApplicationWindow,
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
        let result = window.surface().and_then(|surf| {
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

    /// Return owned effects; no native dialog may be presented in this scope.
    fn poll_osd(
        s: &SurfaceCore,
        window: &gtk::ApplicationWindow,
        physical_size: SurfaceSize,
    ) -> Option<(TurnOutcome, Option<String>)> {
        let mut state = match s.state.try_borrow_mut() {
            Ok(state) => state,
            Err(_) => {
                log::warn!("gtk tick: state busy before poll, skipping frame");
                return None;
            }
        };
        let outcome = drive_turn(&mut GtkTurn {
            core: s,
            window,
            state: &mut state,
            physical_size,
        });
        let modal = s.osd.borrow_mut().take_modal();
        Some((outcome, modal))
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

#[cfg(test)]
mod wake_tests {
    use super::*;
    use std::{cell::Cell, time::Duration};

    struct Harness {
        window: gtk::ApplicationWindow,
        state: Rc<RefCell<State>>,
        surface: Surface,
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
            let state = crate::osd::history_dialog_tests::model_state();
            let factory = state.borrow().ctx.gpu_factory.clone();
            let surface = Surface::bind(&window, &picture, state.clone(), factory);
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
        assert!(!s.renderer.borrow().attached());
        assert_eq!(s.renderer.borrow().output_generation(), 0);
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
        assert!(!h.surface.borrow().renderer.borrow().attached());
        assert!(h.surface.borrow().picture.paintable().is_none());
    }

    #[test]
    fn native_shared_osd_deadline_expires_loaded_hidden_without_renderer() {
        use nerust_gui_shell::session::{SessionHandle, notification::FailureOrigin};
        use nerust_system_traits::factory::{load::MediaObject, settings::FactorySettingsView};

        let Some(h) = Harness::new() else { return };
        let factory = nerust_nes_factory::NesFactory::boxed();
        let system_id = factory.system_id();
        let view = FactorySettingsView {
            language: Default::default(),
            system_config: factory
                .as_system_defaults()
                .unwrap()
                .default_system_settings(),
        };
        let request = factory
            .resolve_load_request(&view, factory.default_load_options())
            .unwrap();
        let paths = nerust_gui_runtime::settings::SettingsPaths::from_root(
            std::env::temp_dir().join(format!("nerust-gtk-wake-loaded-{}", std::process::id())),
        );
        let root = paths.settings_file.parent().unwrap().to_path_buf();
        {
            let mut state = h.state.borrow_mut();
            state.session = SessionHandle::new_with_settings_paths(
                nerust_gui_runtime::settings::HostBackendCapabilities {
                    window: nerust_gui_runtime::settings::HostWindowCapabilities {
                        remembers_window_size: false,
                        supports_fullscreen_default: true,
                        supports_scaling: true,
                    },
                    presentation: None,
                },
                std::sync::Arc::new(nerust_gui_shell::registry::SystemRegistry::new(vec![
                    factory.into(),
                ])),
                std::sync::Arc::new(nerust_core_traits::audio::AudioBackendRegistry::new()),
                paths,
            )
            .unwrap();
            let mut settings = state.session.settings_snapshot().clone();
            settings.local.notifications.info_enabled = false;
            settings.local.notifications.warning_enabled = false;
            settings.local.notifications.error_modal = false;
            state.session.apply_settings(settings).unwrap();
            state.session.set_active_system(system_id.as_ref()).unwrap();
            let mut rom = vec![0u8; 16 + 16384 + 8192];
            rom[..6].copy_from_slice(&[b'N', b'E', b'S', 0x1a, 1, 1]);
            rom[16..19].copy_from_slice(&[0x4c, 0, 0x80]);
            for vector in [0x3ffa, 0x3ffc, 0x3ffe] {
                rom[16 + vector..18 + vector].copy_from_slice(&[0, 0x80]);
            }
            state
                .session
                .load_resolved(MediaObject::new(None, rom), request)
                .unwrap();
            assert!(state.session.tick_metrics().loaded);
            assert!(state.render_profile().is_some());
            state
                .session
                .report_external_error(FailureOrigin::Host, "expires hidden".into());
        }
        assert!(h.surface.tick());
        let deadline = h
            .surface
            .borrow()
            .wake
            .borrow()
            .deadline
            .expect("shared OSD expiry scheduled");
        assert!(deadline > Instant::now());
        assert!(!h.window.is_realized());
        // Error rows have a real five-second TTL; dispatch GLib, never advance
        // a fake clock or invoke tick manually to make the notification expire.
        let limit = Instant::now() + Duration::from_secs(8);
        while h.surface.borrow().wake.borrow().source.is_some() {
            assert!(Instant::now() < limit, "hidden OSD expiry failed to settle");
            glib::MainContext::default().iteration(false);
            std::thread::yield_now();
        }
        assert!(Instant::now() >= deadline);
        let s = h.surface.borrow();
        assert!(s.wake.borrow().deadline.is_none());
        assert!(
            s.retry.borrow().deadline().is_none(),
            "no renderer attach attempted"
        );
        assert_eq!(s.renderer.borrow().output_generation(), 0);
        assert!(!s.renderer.borrow().attached());
        assert!(s.picture.paintable().is_none());
        drop(s);
        h.state.borrow_mut().session.unload().unwrap();
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn native_modal_map_can_borrow_state_surface_and_osd_then_ack() {
        use nerust_frontend_kernel::pause::PauseOwner;
        use nerust_gui_shell::session::notification::FailureOrigin;

        let Some(h) = Harness::new() else { return };
        h.state
            .borrow_mut()
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
                            assert!(s.osd.try_borrow_mut().is_ok());
                            assert!(s.renderer.try_borrow_mut().is_ok());
                            let state = state.borrow_mut();
                            assert!(state.pause_policy.owns(PauseOwner::Modal));
                            // The firing source was cleared before entering GTK.
                            assert!(s.wake.borrow().source.is_none());
                            mapped.set(mapped.get() + 1);
                        }
                    });
                    dialogs.borrow_mut().push(dialog);
                }
            }
        });
        h.state
            .borrow_mut()
            .session
            .report_external_error(FailureOrigin::Host, "first".into());
        h.schedule(Some(Instant::now()));
        dispatch_until(|| mapped.get() == 1);
        h.state
            .borrow_mut()
            .session
            .report_external_error(FailureOrigin::Host, "second".into());
        assert!(h.surface.tick());
        assert_eq!(mapped.get(), 1, "only one dialog until ACK");
        let first = dialogs.borrow()[0].clone();
        first.response(gtk::ResponseType::Ok);
        assert_eq!(
            mapped.get(),
            2,
            "ACK presents the next batch without turn borrows"
        );
        assert!(h.state.borrow().pause_policy.owns(PauseOwner::Modal));
        let second = dialogs.borrow()[1].clone();
        second.response(gtk::ResponseType::Ok);
        assert!(!h.state.borrow().pause_policy.owns(PauseOwner::Modal));
        assert!(h.state.borrow().pause_policy.owns(PauseOwner::Focus));
        toplevels.disconnect(signal);
    }
}

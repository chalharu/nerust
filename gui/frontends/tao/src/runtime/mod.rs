mod dispatch;
mod drain;
pub(crate) mod host;

use nerust_frontend_host::{bootstrap::Boot, event::EventSink, viewport::ViewportPresent};
use raw_window_handle::HasDisplayHandle;
use std::{path::Path, sync::Arc};
#[cfg(target_os = "macos")]
use tao::platform::macos::EventLoopExtMacOS;
use tao::{
    event::{Event, StartCause},
    event_loop::{ControlFlow, EventLoop, EventLoopBuilder},
};

use self::host::{HostAction, HostState};
use crate::event::{TaoEventSink, UserEvent};
use crate::menu::imp::AppMenu;
use crate::ports::{NativeDialogs, TaoWindowOpener};

pub(crate) struct WindowRuntime {
    event_loop: Option<EventLoop<UserEvent>>,
    event_sink: Option<Arc<dyn EventSink + Send + Sync>>,
    host: HostState,
    /// Native target the surface is attached to (`None` = toplevel).
    surface_xid: Option<u64>,
}

impl WindowRuntime {
    pub(crate) fn new(boot: Boot) -> Self {
        let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
        #[cfg(target_os = "macos")]
        let event_loop = {
            let mut event_loop = event_loop;
            event_loop.set_activate_ignoring_other_apps(true);
            event_loop
        };
        let event_sink: Arc<dyn EventSink + Send + Sync> =
            Arc::new(TaoEventSink(event_loop.create_proxy()));
        let app_menu = AppMenu::new(event_sink.clone());

        let host = HostState::new(boot, Box::new(app_menu), std::sync::Arc::new(NativeDialogs));

        Self {
            event_loop: Some(event_loop),
            event_sink: Some(event_sink),
            host,
            surface_xid: None,
        }
    }

    /// Live-follow pulse: parked with no wakeups unless a viewer
    /// wants live ticks; arming wakes it, disarming re-parks it.
    fn spawn_tick_thread(
        tick_sink: Arc<dyn EventSink + Send + Sync>,
        tick_armed: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> Option<std::thread::Thread> {
        let (thread_tx, thread_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = thread_tx.send(std::thread::current());
            use std::sync::atomic::Ordering;
            loop {
                if tick_armed.load(Ordering::Acquire) {
                    std::thread::sleep(std::time::Duration::from_millis(250));
                    if !tick_sink.send(UserEvent::LiveTick) {
                        break;
                    }
                } else {
                    std::thread::park();
                }
            }
        });
        thread_rx.recv().ok()
    }

    pub(crate) fn load_path(&mut self, path: &Path) -> bool {
        match self.host.load_path(path) {
            Ok(()) => {
                self.recreate_renderer();
                true
            }
            Err(detail) => {
                // Startup runs before the main window exists: standalone dialog only.
                crate::ports::show_rom_load_error(
                    self.host.dialog_titles().rom_load_failed,
                    &detail,
                );
                false
            }
        }
    }

    pub(crate) fn run(mut self) {
        self.host.resume_session();
        let event_loop = self
            .event_loop
            .take()
            .expect("WindowRuntime::run() called more than once");

        if let Some(thread) = Self::spawn_tick_thread(
            self.event_sink.take().expect("native event sink"),
            self.host.live_armed(),
        ) {
            self.host.register_tick_thread(thread);
        }

        event_loop.run(move |event, event_loop, control_flow| match event {
            Event::NewEvents(StartCause::Init) => {
                let opener = TaoWindowOpener::new(event_loop);
                self.host.ensure_window(&opener);
                self.recreate_renderer();
                *control_flow = ControlFlow::Wait;
            }
            Event::WindowEvent {
                event, window_id, ..
            } if self.host.is_window(window_id) => {
                let opener = TaoWindowOpener::new(event_loop);
                self.on_main_event(&opener, event, control_flow);
            }
            Event::WindowEvent {
                event, window_id, ..
            } => {
                if let Some(kind) = self.host.aux_kind_of(window_id) {
                    let opener = TaoWindowOpener::new(event_loop);
                    self.on_aux_event(kind, event, &opener);
                }
            }
            Event::RedrawRequested(window_id) if self.host.is_window(window_id) => self.on_update(),
            Event::RedrawRequested(window_id) => {
                // Debugger redraws also drain; the rest only render.
                if let Some(kind) = self.host.aux_kind_of(window_id) {
                    if kind == crate::aux::focus::AuxKind::Debug {
                        let opener = TaoWindowOpener::new(event_loop);
                        self.on_debug_redrawn(&opener);
                    } else {
                        self.on_aux_redrawn(kind);
                    }
                }
            }
            Event::MainEventsCleared => self.host.update_control_flow(control_flow),
            Event::UserEvent(command) => {
                let opener = TaoWindowOpener::new(event_loop);
                let action = self.host.on_event(command, &opener);
                match action {
                    HostAction::None => (),
                    HostAction::RomLoaded => self.recreate_renderer(),
                    HostAction::Exit => {
                        self.begin_close();
                        *control_flow = ControlFlow::Exit
                    }
                }
            }
            Event::LoopDestroyed => self.host.clear_event_handler(),
            _ => (),
        });
    }

    fn on_update(&mut self) {
        if !self.host.render_ready() {
            return;
        }
        if !self.host.viewport_attached() {
            self.recreate_renderer();
        }
        let Some((width, height)) = self.host.window_surface_size() else {
            return;
        };
        if width == 0 || height == 0 {
            return;
        }

        // Keep the output sized to the current target: target change needs a full recreate.
        let current_xid = self.host.surface_xid();
        if self.surface_xid != current_xid {
            self.recreate_renderer();
            return;
        }
        // No notification drain here: `update_control_flow` drains every pass.
        let client_height = self.host.client_height();
        match self.host.viewport_present(width, height, client_height) {
            ViewportPresent::Presented => {
                self.host.refresh_title_after_present();
            }
            ViewportPresent::NoPipeline => {
                self.recreate_renderer();
            }
            ViewportPresent::Skipped => {}
        }
    }

    fn recreate_renderer(&mut self) {
        if !self.host.render_ready() {
            return;
        }
        // Owned display handle out; the surface target below only borrows the host.
        let display = self
            .host
            .with_main_window(|window| window.display_handle().ok().map(|handle| handle.as_raw()));
        let Some(Some(raw_display)) = display else {
            if display.is_some() {
                log::error!("failed to get display handle");
                self.host
                    .render_failed("Failed to get display handle".into());
            }
            return;
        };
        let Some((raw_window, (width, height), xid)) = self.host.surface_target() else {
            return;
        };
        let region = wakui::viewport::ViewportRegion {
            window: raw_window,
            display: raw_display,
            width,
            height,
        };
        match self.host.viewport_sync(region, true) {
            Ok(true) => {
                self.surface_xid = xid;
            }
            Ok(false) => {}
            Err(detail) => {
                log::error!("failed to create renderer: {detail}");
                self.host
                    .render_failed(format!("Failed to create renderer: {detail}"));
            }
        }
    }
}

impl Drop for WindowRuntime {
    fn drop(&mut self) {
        self.host.viewport_drop();
    }
}

impl WindowRuntime {
    /// Ordered exit teardown: the surface dies before the native owner it
    /// borrows, on every close path. Idempotent via `prepare_close`.
    fn begin_close(&mut self) {
        self.host.viewport_drop();
        self.host.prepare_close();
        self.surface_xid = None;
    }
}

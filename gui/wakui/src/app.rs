//! Framework app boundary: the single policy owner as a trait.
//!
//! View definitions implement [`FrontendApp`] over the session; native
//! frontends hold it behind [`Application`] and never name app, shell, or
//! ViewModel types. Every method is object-safe so the composition root
//! can inject the concrete app as a trait object.

use std::{path::Path, time::Instant};

use crate::{
    control::{ControlEffect, ControlEvent, ControlScreen},
    viewport::ViewportRegion,
};

use crate::{
    aux::AuxRegistryPort,
    focus::{FocusEffect, FocusEvent},
    mailbox::WindowMailbox,
    menu::{MenuCapabilities, Menubar},
    osd::{ModalAdvance, ModalToken},
    viewport::{ViewportImage, ViewportPresent},
    window::{Window, WindowChromeSync, WindowEffects},
};
use crate::{
    debugger::{DebugHandle, DebuggerEffects, DebuggerEvent, DebuggerLiveEffects, DebuggerScope},
    event::EventHandler,
    session::{SessionError, SessionEvent, SessionOutcome},
};

/// Opaque close outcome: the native handle leaves as type-erased storage
/// so frontends downcast on their side of the boundary.
pub struct WidgetCloseOutcome {
    pub native: Option<Box<dyn std::any::Any>>,
    pub effects: WindowEffects,
}

/// Built bundle handed from the composition root to a frontend. The app is
/// already constructed (session, factory, loader); frontend entry points
/// receive only this struct. `K` is the view-side aux kind.
///
/// Build it with [`Application::builder`]: the launch document (a ROM path
/// for the emulator, any file for a third-party app) travels inside the
/// bundle, so every backend honors it the same way. The bundle is consumed
/// once by the backend's `run`.
pub struct Application<K: Copy + Eq + std::hash::Hash> {
    app: Box<dyn FrontendApp<K>>,
    initial_document: Option<std::path::PathBuf>,
}

/// Builder for [`Application`]: the single place launch options enter the
/// bundle. Chainable; `build()` hands the bundle to a backend's `run`.
pub struct ApplicationBuilder<K: Copy + Eq + std::hash::Hash> {
    app: Box<dyn FrontendApp<K>>,
    initial_document: Option<std::path::PathBuf>,
}

impl<K: Copy + Eq + std::hash::Hash> Application<K> {
    /// Start building over an already-constructed app.
    pub fn builder(app: Box<dyn FrontendApp<K>>) -> ApplicationBuilder<K> {
        ApplicationBuilder {
            app,
            initial_document: None,
        }
    }

    /// Shortcut for a builder with no launch document.
    pub fn new(app: Box<dyn FrontendApp<K>>) -> Self {
        Self::builder(app).build()
    }

    /// Borrow the policy owner back (e.g. for pre-run probes in tests).
    pub fn app_mut(&mut self) -> &mut dyn FrontendApp<K> {
        &mut *self.app
    }

    /// Release the policy owner; the backend calls this once at startup.
    pub fn into_app(self) -> Box<dyn FrontendApp<K>> {
        self.app
    }

    /// Launch document, if one was supplied.
    pub fn initial_document(&self) -> Option<&Path> {
        self.initial_document.as_deref()
    }

    /// Take the launch document; backends drain it once at startup.
    pub fn take_initial_document(&mut self) -> Option<std::path::PathBuf> {
        self.initial_document.take()
    }

    /// Replace the launch document after the build (composition-root only).
    pub fn set_initial_document(&mut self, path: Option<std::path::PathBuf>) {
        self.initial_document = path;
    }
}

impl<K: Copy + Eq + std::hash::Hash> ApplicationBuilder<K> {
    /// Start building over an already-constructed app.
    pub fn new(app: Box<dyn FrontendApp<K>>) -> Self {
        Application::<K>::builder(app)
    }

    /// Launch document to open at startup (ROM path, project file, ...).
    pub fn initial_document(mut self, path: impl Into<std::path::PathBuf>) -> Self {
        self.initial_document = Some(path.into());
        self
    }

    /// Optional launch document; `None` keeps the current value.
    pub fn maybe_initial_document(mut self, path: Option<std::path::PathBuf>) -> Self {
        if let Some(path) = path {
            self.initial_document = Some(path);
        }
        self
    }

    /// Hand the bundle to a backend's `run`.
    pub fn build(self) -> Application<K> {
        Application {
            app: self.app,
            initial_document: self.initial_document,
        }
    }
}

/// The single policy owner, behind a trait object. Native adapters drive
/// one [`Application::into_app`] through these narrow ports; all ViewModel contact
/// stays inside the implementation. `K` is the view-side aux kind
/// (debug/settings/history/...); the framework never names it.
pub trait FrontendApp<K: Copy + Eq + std::hash::Hash>: EventHandler<K> {
    // -- lifecycle -------------------------------------------------------
    fn load_rom(&mut self, path: &Path) -> Result<(), String>;
    fn unload(&mut self);
    fn begin_close(&mut self) -> bool;
    fn finish_close(&mut self);
    fn close_exited(&self) -> bool;
    fn fatal_message(&self) -> Option<String>;
    fn fail_window(&mut self, detail: String);
    fn resume_session(&mut self);
    fn swap_frame_buffer(&mut self);

    // -- session queries (replacing direct session borrows) --------------
    fn tick_loaded(&self) -> bool;
    fn tick_paused(&self) -> bool;
    fn active_slot_id(&self) -> Option<u64>;
    fn clear_keys(&mut self);
    fn update_window_size(&mut self, width: u32, height: u32) -> Result<(), String>;
    fn pause_session(&mut self);
    fn resume_session_command(&mut self);
    fn toggle_pause_session(&mut self);
    fn reset_session(&mut self);
    fn save_active_slot(&mut self) -> bool;
    fn load_active_slot(&mut self) -> bool;
    fn load_slot(&mut self, id: u64) -> bool;
    fn save_slot(&mut self, id: u64);
    fn delete_slot(&mut self, id: u64);
    fn select_slot(&mut self, id: u64);
    fn create_slot(&mut self);
    fn select_next_slot(&mut self);
    fn select_previous_slot(&mut self);
    fn set_pause_owner(&mut self, owner: crate::pause::PauseOwner, active: bool);
    fn owns_pause(&self, owner: crate::pause::PauseOwner) -> bool;
    fn pause_held(&self) -> bool;
    fn reconcile_pause(&mut self);
    fn dispatch(&mut self, event: SessionEvent) -> Result<SessionOutcome, SessionError>;

    // -- chrome, menus, titles -------------------------------------------
    fn dialog_titles(&self) -> crate::menu::DialogTitles;
    fn rom_dialog_filters(&self) -> Vec<(String, Vec<String>)>;
    fn menubar_update(
        &mut self,
        settings_open: bool,
        capabilities: MenuCapabilities,
    ) -> Option<Menubar>;
    fn main_window(&self) -> &Window;
    fn window_usable(&self) -> bool;
    fn title_update(&mut self) -> Option<String>;
    fn title_refresh_due(&mut self, now: Instant) -> bool;
    fn refresh_window_title(&mut self) -> Option<String>;
    fn observe_fullscreen(&mut self, native: bool) -> bool;
    fn fullscreen_request(&mut self, native: bool) -> Option<bool>;
    fn persist_fullscreen_default(&mut self, fullscreen: bool) -> Result<bool, String>;
    fn window_size(&self) -> (f32, f32);
    fn window_scale_factor(&self) -> Option<u32>;
    fn window_chrome_sync(&mut self, native_fullscreen: bool) -> WindowChromeSync;
    fn remembered_window_size(&self) -> Option<(u32, u32)>;
    fn liveness_inputs(&self) -> (bool, bool);

    // -- focus -------------------------------------------------------------
    fn focus_event(&mut self, event: FocusEvent) -> Option<FocusEffect>;
    fn main_active(&self) -> bool;
    fn note_main_arrival(&mut self) -> bool;
    fn note_main_departure(&mut self);
    /// View-side resolves `id` to its kind and applies the resume protocol.
    fn drive_aux_focus(&mut self, id: crate::aux::AuxId, focused: bool) -> bool;
    fn release_focus_pause(&mut self);

    // -- aux widget windows (id-keyed; sessions owned view-side) ---------
    fn reserve_widget(
        &mut self,
        registry: &mut dyn AuxRegistryPort<K>,
        kind: K,
    ) -> Result<Option<crate::aux::OpenRequest>, String>;
    /// Collect and consume deferred aux open requests (e.g. the PPU viewer
    /// requested from a debugger gesture). Flavors open each kind through
    /// the normal `reserve_widget`/`prepare_widget` path.
    fn drain_open_requests(&mut self) -> Vec<K>;
    fn cancel_widget(
        &mut self,
        registry: &mut dyn AuxRegistryPort<K>,
        id: crate::aux::AuxId,
    ) -> bool;
    fn prepare_widget(
        &mut self,
        registry: &mut dyn AuxRegistryPort<K>,
        id: crate::aux::AuxId,
    ) -> Result<bool, String>;
    fn widget_screen(&self, id: crate::aux::AuxId) -> Option<ControlScreen>;
    fn widget_window(&self, id: crate::aux::AuxId) -> Option<Window>;
    fn widget_mailbox(&self, id: crate::aux::AuxId) -> Option<WindowMailbox>;
    fn widget_dispatch(&mut self, id: crate::aux::AuxId, event: ControlEvent)
    -> Vec<ControlEffect>;
    fn widget_submit_picker(
        &mut self,
        id: crate::aux::AuxId,
        target: crate::control::ControlId,
        request: crate::control::Id,
        payload: Option<std::path::PathBuf>,
    );
    fn widget_refresh(&mut self, id: crate::aux::AuxId);
    fn close_widget(
        &mut self,
        registry: &mut dyn AuxRegistryPort<K>,
        id: crate::aux::AuxId,
        close: bool,
    ) -> Option<WidgetCloseOutcome>;
    fn settings_closed(&mut self) -> bool;
    fn report_host_error(&mut self, detail: String);
    fn history_changed(&self) -> bool;
    /// Whether a settings widget is currently open. Flavors gate menus and
    /// session commands on this without naming the settings kind.
    fn settings_open(&self) -> bool;

    // -- debugger (widget window; bridge owned view-side) -------------------
    // The debugger is a generic Controls widget like settings/history/PPU.
    // Flavors open it via `reserve_widget`/`prepare_widget`, push
    // `widget_screen`s, and route `widget_dispatch` events; all bridge and
    // drain state lives view-side behind these narrow ports.
    /// Open debugger widget id, if any.
    fn debug_widget(&self) -> Option<crate::aux::AuxId>;
    /// Bridge handle behind a debugger widget id, if still live.
    fn debug_handle_for(&self, id: crate::aux::AuxId) -> Option<DebugHandle>;
    fn close_debug_flights(&mut self);
    fn debug_gesture(
        &mut self,
        handle: DebugHandle,
        inputs: Vec<DebuggerEvent>,
        event: DebuggerEvent,
    ) -> DebuggerEffects;
    fn debug_refresh(&mut self, handle: DebugHandle, scope: DebuggerScope);
    fn debug_open_refresh(&mut self, handle: DebugHandle);
    fn debug_poll(&mut self, handle: DebugHandle) -> crate::loop_driver::WakeNeed;
    fn debug_poll_live(&mut self, handle: DebugHandle, visible: bool) -> DebuggerLiveEffects;
    fn debug_live_tick(&mut self, handle: DebugHandle);
    fn debug_drain_pending(&self, handle: DebugHandle) -> bool;
    fn debug_reset_if_replaced(&mut self, handle: DebugHandle) -> bool;
    /// Chrome gate: a pause change since the last drain needs a viewer refresh.
    fn chrome_drain_due(&self) -> bool;
    /// Generic aux open latch + window position memory, keyed by view-side kind.
    /// View definitions map their kinds (debugger/PPU/...) to these; native
    /// code never names the kinds.
    fn take_open_request(&mut self, kind: K) -> bool;
    fn request_open(&mut self, kind: K);
    fn window_pos(&self, kind: K) -> Option<(i32, i32)>;
    fn remember_window_pos(&mut self, kind: K, x: i32, y: i32);
    /// Live-tick arming for the native timer thread. The schedule lives
    /// view-side; native code parks/unparks its thread on this flag.
    fn live_armed(&self) -> std::sync::Arc<std::sync::atomic::AtomicBool>;
    fn register_tick_thread(&mut self, thread: std::thread::Thread);
    fn next_live_tick(&self) -> Option<std::time::Instant>;
    fn live_due(&mut self, now: std::time::Instant) -> bool;

    // -- OSD / notifications ----------------------------------------------
    fn pump_osd(&mut self, now: Instant) -> crate::osd::OsdTurn;
    fn attach_osd_modal_receiver(
        &mut self,
        token: ModalToken,
        receiver: std::sync::mpsc::Receiver<()>,
    ) -> bool;
    fn acknowledge_osd_modal(&mut self, token: ModalToken) -> ModalAdvance;
    fn osd_modal_pending(&self) -> bool;
    fn osd_flight_pending(&self) -> bool;
    fn osd_pending(&self) -> usize;
    fn osd_wakeup(&self, now: Instant) -> Option<Instant>;
    fn note_redraw(&mut self);
    fn redraw_needed(&self) -> bool;
    fn invalidate_osd(&mut self);

    // -- viewport / render --------------------------------------------------
    fn viewport_sync(&mut self, region: ViewportRegion, force: bool) -> Result<bool, String>;
    fn viewport_present(&mut self, width: u32, height: u32, client_height: u32) -> ViewportPresent;
    fn viewport_capture(&mut self, width: u32, height: u32) -> Option<ViewportImage>;
    fn sync_osd_viewport(&mut self, width: u32, height: u32, client_height: u32);
    fn viewport_drop(&mut self);
    fn viewport_attached(&self) -> bool;
    fn viewport_generation(&self) -> u64;
    fn render_ready(&self) -> bool;
    fn render_failed(&mut self, detail: String);
    fn render_retry_deadline(&self) -> Option<Instant>;
    fn take_render_retry(&mut self, now: Instant) -> bool;
    fn render_succeeded(&mut self);
}

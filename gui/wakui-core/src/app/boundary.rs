//! Framework app boundary: the single policy owner as a trait.
//!
//! View definitions implement [`FrontendApp`] over the session; native
//! frontends hold it behind [`Application`] and never name app, shell, or
//! ViewModel types. Every method is object-safe so the composition root
//! can inject the concrete app as a trait object.

use std::{path::Path, time::Instant};

use crate::{
    chrome::viewport::ViewportRegion,
    scene::control::{ControlEffect, ControlEvent, ControlScreen},
};

use crate::{
    chrome::aux::AuxRegistryPort,
    chrome::dialog::{DialogTurn, ModalAdvance, ModalToken},
    chrome::mailbox::WindowMailbox,
    chrome::menu::{MenuCapabilities, Menubar},
    chrome::viewport::{ViewportImage, ViewportPresent},
    chrome::window::{Window, WindowChromeSync, WindowEffects},
    input::focus::FocusEvent,
};
use crate::{
    input::event::EventHandler,
    input::session::{SessionError, SessionEvent, SessionOutcome},
};

/// Opaque close outcome: the native handle leaves as type-erased storage
/// so frontends downcast on their side of the boundary.
pub struct WidgetCloseOutcome {
    pub native: Option<Box<dyn std::any::Any>>,
    pub effects: WindowEffects,
}

/// Built bundle handed from the composition root to a frontend. The app is
/// already constructed (content handle, factory, loader); frontend entry points
/// receive only this struct. `K` is the view-side aux kind, `A` the
/// launch-arguments type.
///
/// Build it with [`Application::builder`]: launch arguments travel inside
/// the bundle, so no backend reads the process argv on its own. The
/// framework never interprets them; each backend just applies its platform
/// file-open convention through [`LaunchArguments`] (tao opens the first
/// file at startup, gtk routes them through the toolkit open events). The
/// bundle is consumed once by the backend's `run`.
///
/// `A` defaults to `()` (no arguments); apps with richer CLIs define their
/// own arguments struct. The builder starts untyped (`()`) and each
/// `.arguments()` call swaps to a differently-typed builder (typestate, no
/// `Default` bound anywhere); the [`LaunchArguments`] bound is checked at
/// the backend's `run`, so the wrong bundle never compiles.
pub struct Application<K: Copy + Eq + std::hash::Hash, A = ()> {
    app: Box<dyn FrontendApp<K>>,
    arguments: A,
}

/// Launch arguments carried in the bundle. Backends read the file operands
/// through this trait; the framework never interprets them. Flags were
/// already consumed upstream, so only file operands travel here.
pub trait LaunchArguments {
    /// File operands to open at startup.
    fn files(&self) -> Vec<String>;
}

impl LaunchArguments for Vec<String> {
    fn files(&self) -> Vec<String> {
        self.clone()
    }
}

impl LaunchArguments for () {
    fn files(&self) -> Vec<String> {
        Vec::new()
    }
}

/// Builder for [`Application`]: the single place launch arguments enter the
/// bundle. Chainable; `build()` hands the bundle to a backend's `run`.
///
/// Typestate: starts as `ApplicationBuilder<K, ()>`; each `.arguments()`
/// call swaps to a differently-typed builder. No `Default` bound is needed
/// at any step.
pub struct ApplicationBuilder<K: Copy + Eq + std::hash::Hash, A = ()> {
    app: Box<dyn FrontendApp<K>>,
    arguments: A,
}

impl<K: Copy + Eq + std::hash::Hash> Application<K, ()> {
    /// Start building over an already-constructed app, with no arguments.
    pub fn builder(app: Box<dyn FrontendApp<K>>) -> ApplicationBuilder<K, ()> {
        ApplicationBuilder::new(app)
    }

    /// Bundle an app with no launch arguments.
    pub fn new(app: Box<dyn FrontendApp<K>>) -> Self {
        Self { app, arguments: () }
    }
}

impl<K: Copy + Eq + std::hash::Hash, A> Application<K, A> {
    /// Borrow the policy owner back (e.g. for pre-run probes in tests).
    pub fn app_mut(&mut self) -> &mut dyn FrontendApp<K> {
        &mut *self.app
    }

    /// Release the policy owner; the backend calls this once at startup.
    pub fn into_app(self) -> Box<dyn FrontendApp<K>> {
        self.app
    }

    /// Opaque launch arguments. The framework never interprets them;
    /// backends read them through [`LaunchArguments`].
    pub fn arguments(&self) -> &A {
        &self.arguments
    }
}

impl<K: Copy + Eq + std::hash::Hash> ApplicationBuilder<K, ()> {
    /// Start building over an already-constructed app, with no arguments.
    pub fn new(app: Box<dyn FrontendApp<K>>) -> Self {
        Self { app, arguments: () }
    }

    /// Append one launch argument, swapping to `Vec<String>` arguments.
    pub fn argument(self, argument: impl Into<String>) -> ApplicationBuilder<K, Vec<String>> {
        ApplicationBuilder {
            app: self.app,
            arguments: vec![argument.into()],
        }
    }
}

impl<K: Copy + Eq + std::hash::Hash, A> ApplicationBuilder<K, A> {
    /// Set launch arguments, swapping to a differently-typed builder. The
    /// type initializes here (or replaces a previous value).
    pub fn arguments<B>(self, arguments: B) -> ApplicationBuilder<K, B> {
        ApplicationBuilder {
            app: self.app,
            arguments,
        }
    }

    /// Hand the bundle to a backend's `run`.
    pub fn build(self) -> Application<K, A> {
        Application {
            app: self.app,
            arguments: self.arguments,
        }
    }
}

impl<K: Copy + Eq + std::hash::Hash> ApplicationBuilder<K, Vec<String>> {
    /// Append one launch argument (file operand).
    pub fn argument(mut self, argument: impl Into<String>) -> Self {
        self.arguments.push(argument.into());
        self
    }
}

/// The single policy owner, behind a trait object. Native adapters drive
/// one [`Application::into_app`] through these narrow ports; all ViewModel contact
/// stays inside the implementation. `K` is the view-side aux kind
/// (view-defined kinds); the framework never names it.
///
/// File and content ports are generic (open/unload content, file dialogs).
/// Suspension arbitration lives view-side; the framework only transports
/// the underlying notifications (focus events, modal effects, commands).
/// Content verbs and aux kinds live view-side behind separate domain
/// ports; flavors never name them.
pub trait FrontendApp<K: Copy + Eq + std::hash::Hash>: EventHandler<K> {
    // -- lifecycle -------------------------------------------------------
    /// Open content from a file. Content-specific wording stays view-side;
    /// the framework only transports the path and the failure string.
    fn load_file(&mut self, path: &Path) -> Result<(), String>;
    fn unload(&mut self);
    fn begin_close(&mut self) -> bool;
    fn finish_close(&mut self);
    fn close_exited(&self) -> bool;
    fn fatal_message(&self) -> Option<String>;
    fn fail_window(&mut self, detail: String);
    /// Resume suspended content (e.g. at startup after construction).
    fn resume_content(&mut self);
    fn swap_frame_buffer(&mut self);

    // -- content state (generic) ------------------------------------------------
    /// Whether content is present (a document is open).
    fn content_loaded(&self) -> bool;
    /// Whether present content is suspended.
    fn content_suspended(&self) -> bool;
    /// Reset held-input state (e.g. on window close).
    fn clear_input(&mut self);
    fn update_window_size(&mut self, width: u32, height: u32) -> Result<(), String>;
    fn dispatch(&mut self, event: SessionEvent) -> Result<SessionOutcome, SessionError>;

    // -- chrome, menus, titles (generic file dialogs) ---------------------
    fn dialog_titles(&self) -> crate::chrome::menu::FileDialogTitles;
    fn file_dialog_filters(&self) -> Vec<(String, Vec<String>)>;
    fn menubar_update(&mut self, capabilities: MenuCapabilities) -> Option<Menubar>;
    fn main_window(&self) -> &Window;
    fn window_usable(&self) -> bool;
    /// Pull the current title; `Some` only when it changed since the last
    /// pull. Flavors call this at chrome-sync points (handled events,
    /// presented frames): updates are immediate and notification-driven,
    /// never timer-polled. Change detection lives view-side.
    fn title_update(&mut self) -> Option<String>;
    fn refresh_window_title(&mut self) -> Option<String>;
    fn observe_fullscreen(&mut self, native: bool) -> bool;
    fn fullscreen_request(&mut self, native: bool) -> Option<bool>;
    fn persist_fullscreen_default(&mut self, fullscreen: bool) -> Result<bool, String>;
    fn window_size(&self) -> (f32, f32);
    fn window_scale_factor(&self) -> Option<u32>;
    fn window_chrome_sync(&mut self, native_fullscreen: bool) -> WindowChromeSync;
    fn remembered_window_size(&self) -> Option<(u32, u32)>;

    // -- focus -------------------------------------------------------------
    // Notifications only: arrivals, departures, burst ends. Suspension
    // decisions live view-side; the framework pauses nothing.
    fn focus_event(&mut self, event: FocusEvent);
    fn main_active(&self) -> bool;
    fn note_main_arrival(&mut self) -> bool;
    fn note_main_departure(&mut self);
    /// View-side resolves `id` to its kind and applies its focus policy.
    fn drive_aux_focus(&mut self, id: crate::chrome::aux::AuxId, focused: bool) -> bool;

    // -- aux widget windows (id-keyed; sessions owned view-side) ---------
    fn reserve_widget(
        &mut self,
        registry: &mut dyn AuxRegistryPort<K>,
        kind: K,
    ) -> Result<Option<crate::chrome::aux::OpenRequest>, String>;
    /// Collect and consume deferred aux open requests (e.g. a viewer
    /// requested from a widget gesture). Flavors open each kind through
    /// the normal `reserve_widget`/`prepare_widget` path.
    fn drain_open_requests(&mut self) -> Vec<K>;
    fn cancel_widget(
        &mut self,
        registry: &mut dyn AuxRegistryPort<K>,
        id: crate::chrome::aux::AuxId,
    ) -> bool;
    fn prepare_widget(
        &mut self,
        registry: &mut dyn AuxRegistryPort<K>,
        id: crate::chrome::aux::AuxId,
    ) -> Result<bool, String>;
    fn widget_screen(&self, id: crate::chrome::aux::AuxId) -> Option<ControlScreen>;
    fn widget_window(&self, id: crate::chrome::aux::AuxId) -> Option<Window>;
    fn widget_mailbox(&self, id: crate::chrome::aux::AuxId) -> Option<WindowMailbox>;
    fn widget_dispatch(
        &mut self,
        id: crate::chrome::aux::AuxId,
        event: ControlEvent,
    ) -> Vec<ControlEffect>;
    fn widget_submit_picker(
        &mut self,
        id: crate::chrome::aux::AuxId,
        target: crate::scene::control::ControlId,
        request: crate::scene::control::Id,
        payload: Option<std::path::PathBuf>,
    );
    fn widget_refresh(&mut self, id: crate::chrome::aux::AuxId);
    fn close_widget(
        &mut self,
        registry: &mut dyn AuxRegistryPort<K>,
        id: crate::chrome::aux::AuxId,
        close: bool,
    ) -> Option<WidgetCloseOutcome>;
    fn report_host_error(&mut self, detail: String);
    /// Whether a modal dialog currently gates chrome and
    /// commands. Flavors gate nothing themselves; the view side enforces the
    /// guard when it handles events and builds menus.
    fn modal_guard_active(&self) -> bool;

    // -- polled widgets (background work owned view-side) ------------------
    // Widgets are generic Controls.
    // Flavors open them via `reserve_widget`/`prepare_widget`, push
    // `widget_screen`s, and route `widget_dispatch` events; all drain and
    // bridge state lives view-side behind these narrow ports. The framework
    // never names the domain: it only polls the widget
    // the view-side marks as needing background work.
    /// Widget id needing background polling, if any.
    fn polled_widget(&self) -> Option<crate::chrome::aux::AuxId>;
    /// Poll background work for `id`, refresh its cached screen when due.
    fn poll_widget(&mut self, id: crate::chrome::aux::AuxId) -> crate::flow::loop_driver::WakeNeed;
    /// Live-tick follow for `id` (images-only refresh while running).
    fn tick_widget(&mut self, id: crate::chrome::aux::AuxId);
    /// Drop background flights (ordered host teardown).
    fn close_background_flights(&mut self);
    /// Generic aux open latch + window position memory, keyed by view-side kind.
    /// View definitions map their kinds to these; native code never names them.
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

    // -- modal dialogs (generic; transient layers live renderer-side) ------
    fn pump_dialog(&mut self, now: Instant) -> DialogTurn;
    fn attach_dialog_receiver(
        &mut self,
        token: ModalToken,
        receiver: std::sync::mpsc::Receiver<()>,
    ) -> bool;
    fn acknowledge_dialog(&mut self, token: ModalToken) -> ModalAdvance;
    fn dialog_pending(&self) -> bool;
    fn dialog_flight_pending(&self) -> bool;
    fn dialog_queue_len(&self) -> usize;
    fn dialog_wakeup(&self, now: Instant) -> Option<Instant>;
    fn note_redraw(&mut self);
    fn redraw_needed(&self) -> bool;
    fn invalidate_overlay(&mut self);

    // -- viewport / render --------------------------------------------------
    fn viewport_sync(&mut self, region: ViewportRegion, force: bool) -> Result<bool, String>;
    fn viewport_present(&mut self, width: u32, height: u32, client_height: u32) -> ViewportPresent;
    fn viewport_capture(&mut self, width: u32, height: u32) -> Option<ViewportImage>;
    fn sync_overlay_viewport(&mut self, width: u32, height: u32, client_height: u32);
    fn viewport_drop(&mut self);
    fn viewport_attached(&self) -> bool;
    fn viewport_generation(&self) -> u64;
    fn render_ready(&self) -> bool;
    fn render_failed(&mut self, detail: String);
    fn render_retry_deadline(&self) -> Option<Instant>;
    fn take_render_retry(&mut self, now: Instant) -> bool;
    fn render_succeeded(&mut self);
}

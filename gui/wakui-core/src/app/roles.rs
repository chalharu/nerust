//! Policy roles: narrow ports composing the [`crate::app::boundary::FrontendApp`] facade.
//!
//! Each trait owns one concern (lifecycle, content, chrome, focus, aux
//! windows, background polling, modal dialogs, viewport). New code bounds
//! on the narrow port it needs instead of the whole app (high cohesion,
//! low coupling); the facade stays as the single implementor-facing trait.
//!
//! Implementors write one `impl` block per role plus an empty
//! [`crate::app::boundary::FrontendApp`] marker; consumers bound on the
//! narrow port they need.

use std::{path::Path, time::Instant};

use crate::{
    app::boundary::WidgetCloseOutcome,
    chrome::aux::{AuxId, AuxRegistryPort, OpenRequest},
    chrome::dialog::{DialogTurn, ModalAdvance, ModalToken},
    chrome::mailbox::WindowMailbox,
    chrome::menu::{FileDialogTitles, MenuCapabilities, Menubar},
    chrome::viewport::{ViewportImage, ViewportPresent, ViewportRegion},
    chrome::window::{Window, WindowChromeSync},
    flow::loop_driver::WakeNeed,
    input::focus::FocusEvent,
    input::session::{SessionError, SessionEvent, SessionOutcome},
    scene::control::{ControlEffect, ControlEvent, ControlId, ControlScreen, Id},
};

/// App lifecycle: construction-adjacent file handling and close sequencing.
pub trait LifecyclePorts {
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
}

/// Held content state: presence, suspension, and input dispatch.
pub trait ContentPorts {
    /// Whether content is present (a document is open).
    fn content_loaded(&self) -> bool;
    /// Whether present content is suspended.
    fn content_suspended(&self) -> bool;
    /// Reset held-input state (e.g. on window close).
    fn clear_input(&mut self);
    fn update_window_size(&mut self, width: u32, height: u32) -> Result<(), String>;
    fn dispatch(&mut self, event: SessionEvent) -> Result<SessionOutcome, SessionError>;
}

/// Native chrome: menus, titles, and fullscreen (generic file dialogs).
pub trait ChromePorts {
    fn dialog_titles(&self) -> FileDialogTitles;
    fn file_dialog_filters(&self) -> Vec<(String, Vec<String>)>;
    fn menubar_update(&mut self, capabilities: MenuCapabilities) -> Option<Menubar>;
    fn main_window(&self) -> &Window;
    fn window_usable(&self) -> bool;
    /// Pull the current title; `Some` only when it changed since the last
    /// pull. Flavors call this at chrome-sync points (handled events,
    /// presented frames): updates are immediate and notification-driven,
    /// never timer-polled. Change detection lives view-side.
    fn title_update(&mut self) -> Option<String>;
    fn observe_fullscreen(&mut self, native: bool) -> bool;
    fn fullscreen_request(&mut self, native: bool) -> Option<bool>;
    fn persist_fullscreen_default(&mut self, fullscreen: bool) -> Result<bool, String>;
    fn window_size(&self) -> (f32, f32);
    fn window_scale_factor(&self) -> Option<u32>;
    fn window_chrome_sync(&mut self, native_fullscreen: bool) -> WindowChromeSync;
    fn remembered_window_size(&self) -> Option<(u32, u32)>;
}

/// Focus notifications: arrivals, departures, burst ends. Suspension
/// decisions live view-side; the framework pauses nothing.
pub trait FocusPorts {
    // Notifications only. View-side resolves `id` to its kind and applies
    // its focus policy.
    fn focus_event(&mut self, event: FocusEvent);
    fn main_active(&self) -> bool;
    fn note_main_arrival(&mut self) -> bool;
    fn note_main_departure(&mut self);
    /// View-side resolves `id` to its kind and applies its focus policy.
    fn drive_aux_focus(&mut self, id: AuxId, focused: bool) -> bool;
}

/// Auxiliary widget windows: id-keyed sessions owned view-side.
///
/// Widgets are generic Controls. Flavors open them via
/// `reserve_widget`/`prepare_widget`, push `widget_screen`s, and route
/// `widget_dispatch` events.
pub trait AuxPorts<K: Copy + Eq + std::hash::Hash> {
    fn reserve_widget(
        &mut self,
        registry: &mut dyn AuxRegistryPort<K>,
        kind: K,
    ) -> Result<Option<OpenRequest>, String>;
    /// Collect and consume deferred aux open requests (e.g. a viewer
    /// requested from a widget gesture). Flavors open each kind through
    /// the normal `reserve_widget`/`prepare_widget` path.
    fn drain_open_requests(&mut self) -> Vec<K>;
    fn cancel_widget(&mut self, registry: &mut dyn AuxRegistryPort<K>, id: AuxId) -> bool;
    fn prepare_widget(
        &mut self,
        registry: &mut dyn AuxRegistryPort<K>,
        id: AuxId,
    ) -> Result<bool, String>;
    fn widget_screen(&self, id: AuxId) -> Option<ControlScreen>;
    fn widget_window(&self, id: AuxId) -> Option<Window>;
    fn widget_mailbox(&self, id: AuxId) -> Option<WindowMailbox>;
    fn widget_dispatch(&mut self, id: AuxId, event: ControlEvent) -> Vec<ControlEffect>;
    fn widget_submit_picker(
        &mut self,
        id: AuxId,
        target: ControlId,
        request: Id,
        payload: Option<std::path::PathBuf>,
    );
    fn widget_refresh(&mut self, id: AuxId);
    fn close_widget(
        &mut self,
        registry: &mut dyn AuxRegistryPort<K>,
        id: AuxId,
        close: bool,
    ) -> Option<WidgetCloseOutcome>;
    fn report_host_error(&mut self, detail: String);
    /// Whether a modal dialog currently gates chrome and
    /// commands. Flavors gate nothing themselves; the view side enforces the
    /// guard when it handles events and builds menus.
    fn modal_guard_active(&self) -> bool;
    /// Generic aux open latch + window position memory, keyed by view-side kind.
    /// View definitions map their kinds to these; native code never names them.
    fn take_open_request(&mut self, kind: K) -> bool;
    fn request_open(&mut self, kind: K);
    fn window_pos(&self, kind: K) -> Option<(i32, i32)>;
    fn remember_window_pos(&mut self, kind: K, x: i32, y: i32);
}

/// Background work owned view-side: all drain and bridge state lives
/// behind these narrow ports. The framework never names the domain: it
/// only polls the widget the view-side marks as needing background work.
pub trait PollPorts {
    /// Widget id needing background polling, if any.
    fn polled_widget(&self) -> Option<AuxId>;
    /// Poll background work for `id`, refresh its cached screen when due.
    fn poll_widget(&mut self, id: AuxId) -> WakeNeed;
    /// Live-tick follow for `id` (images-only refresh while running).
    fn tick_widget(&mut self, id: AuxId);
    /// Drop background flights (ordered host teardown).
    fn close_background_flights(&mut self);
    /// Live-tick arming for the native timer thread. The schedule lives
    /// view-side; native code parks/unparks its thread on this flag.
    fn live_armed(&self) -> std::sync::Arc<std::sync::atomic::AtomicBool>;
    fn register_tick_thread(&mut self, thread: std::thread::Thread);
    fn next_live_tick(&self) -> Option<Instant>;
    fn live_due(&mut self, now: Instant) -> bool;
}

/// Modal dialogs (generic; transient layers live renderer-side).
pub trait DialogPorts {
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
}

/// Viewport and render seam.
pub trait ViewportPorts {
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
    fn swap_frame_buffer(&mut self);
}

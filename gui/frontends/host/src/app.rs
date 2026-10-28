//! Framework app boundary: the single policy owner as a trait.
//!
//! View definitions implement [`FrontendApp`] over the session; native
//! frontends hold it behind [`Boot`] and never name app, shell, or
//! ViewModel types. Every method is object-safe so the composition root
//! can inject the concrete app as a trait object.

use std::{path::Path, time::Instant};

use wakui::{
    control::{ControlEffect, ControlEvent, ControlScreen},
    viewport::ViewportRegion,
};

use crate::{
    aux::AuxRegistryPort,
    debugger::{
        DebugHandle, DebugSnapshot, DebuggerChrome, DebuggerEffects, DebuggerEvent,
        DebuggerLiveEffects, DebuggerScope, PpuChrome, ScaledPicture,
    },
    event::EventHandler,
    focus::{AuxKind, FocusEffect, FocusEvent},
    menu::{MenuCapabilities, Menubar},
    osd::{ModalAdvance, ModalToken},
    session::{SessionError, SessionEvent, SessionOutcome},
    viewport::{ViewportImage, ViewportPresent},
    widget_session::WindowMailbox,
    window::{Window, WindowChromeSync, WindowEffects},
};

/// Opaque close outcome: the native handle leaves as type-erased storage
/// so frontends downcast on their side of the boundary.
pub struct WidgetCloseOutcome {
    pub native: Option<Box<dyn std::any::Any>>,
    pub effects: WindowEffects,
}

/// Built bundle handed from the composition root to a frontend. The app is
/// already constructed (session, factory, loader); frontend entry points
/// receive only this struct.
pub struct Boot {
    pub app: Box<dyn FrontendApp>,
}

/// The single policy owner, behind a trait object. Native adapters drive
/// one [`Boot::app`] through these narrow ports; all ViewModel contact
/// stays inside the implementation.
pub trait FrontendApp: EventHandler {
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
    fn set_pause_owner(&mut self, owner: wakui::pause::PauseOwner, active: bool);
    fn owns_pause(&self, owner: wakui::pause::PauseOwner) -> bool;
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
    fn drive_aux_focus(&mut self, kind: AuxKind, focused: bool) -> bool;
    fn release_focus_pause(&mut self);

    // -- aux widget windows (id-keyed; sessions owned view-side) ---------
    fn reserve_widget(
        &mut self,
        registry: &mut dyn AuxRegistryPort,
        kind: AuxKind,
    ) -> Result<Option<crate::aux::OpenRequest>, String>;
    fn cancel_widget(&mut self, registry: &mut dyn AuxRegistryPort, id: crate::aux::AuxId) -> bool;
    fn prepare_widget(
        &mut self,
        registry: &mut dyn AuxRegistryPort,
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
        target: wakui::control::ControlId,
        request: wakui::control::Id,
        payload: Option<std::path::PathBuf>,
    );
    fn widget_refresh(&mut self, id: crate::aux::AuxId);
    fn close_widget(
        &mut self,
        registry: &mut dyn AuxRegistryPort,
        id: crate::aux::AuxId,
        close: bool,
    ) -> Option<WidgetCloseOutcome>;
    fn settings_closed(&mut self) -> bool;
    fn report_host_error(&mut self, detail: String);
    fn history_changed(&self) -> bool;

    // -- debugger (handle-keyed; bridges owned view-side) ----------------
    fn open_debug(&mut self) -> DebugHandle;
    fn close_debug(&mut self, handle: DebugHandle);
    fn debug_snapshot(&self, handle: DebugHandle) -> Option<DebugSnapshot>;
    fn debug_pictures(&self, handle: DebugHandle) -> Vec<ScaledPicture>;
    fn debug_panels(&self, handle: DebugHandle) -> Option<String>;
    fn debug_paused(&self, handle: DebugHandle) -> bool;
    fn debug_generation(&self, handle: DebugHandle) -> u64;
    fn debug_gesture(
        &mut self,
        handle: DebugHandle,
        inputs: Vec<DebuggerEvent>,
        event: DebuggerEvent,
        need_images: bool,
    ) -> DebuggerEffects;
    fn debug_refresh(&mut self, handle: DebugHandle, scope: DebuggerScope, need_images: bool);
    fn debug_open_refresh(&mut self, handle: DebugHandle, need_images: bool);
    fn debug_poll(&mut self, handle: DebugHandle, ppu_open: bool) -> wakui::loop_driver::WakeNeed;
    fn debug_poll_live(
        &mut self,
        handle: DebugHandle,
        visible: bool,
        need_images: bool,
    ) -> DebuggerLiveEffects;
    fn debug_live_tick(&mut self, handle: DebugHandle, ppu_open: bool);
    fn debug_drain_pending(&self, handle: DebugHandle) -> bool;
    fn debug_reset_if_replaced(&mut self, handle: DebugHandle) -> bool;
    fn debugger_chrome(&self) -> DebuggerChrome;
    fn ppu_chrome(&self) -> PpuChrome;
    fn debugger_window_title(&self) -> String;
    fn debug_hover_line(
        &self,
        handle: DebugHandle,
        label: &str,
        width: u32,
        height: u32,
        x: f64,
        y: f64,
    ) -> String;
    fn take_debug_open_request(&mut self) -> bool;
    fn take_ppu_open_request(&mut self) -> bool;
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

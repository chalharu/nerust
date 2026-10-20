use nerust_gui_runtime::{
    settings::{BackendPresentationCapabilities, HostBackendCapabilities, HostWindowCapabilities},
    shell::NativeShellState,
};
use nerust_gui_shell::{
    context::FrontendContext,
    debug::{DrainFlight, LiveImagesFlight},
    session::{SessionHandle, session_query::WindowSize},
};
use nerust_render_traits::{SurfaceSize, renderer::GpuFactory};
use nerust_settings_core::model::app_state::RememberedWindowSize;
#[cfg(test)]
use std::path::PathBuf;
use std::{rc::Rc, sync::Arc};
use tao::{
    dpi::{LogicalSize as TaoLogicalSize, PhysicalSize as TaoPhysicalSize},
    event::ElementState,
    window::{Fullscreen, Window as TaoWindow, WindowBuilder, WindowId},
};

use crate::menu::MenuSink;

use super::debug_windows::DebugWindows;
use crate::aux::focus::{AuxKind, FocusCoordinator, FocusEffect, FocusEvent};

pub(crate) mod close;
mod commands;
mod focus;
mod osd;
use osd::ModalFlight;
mod presentation;
mod settings;
mod window;

use crate::aux::osd::HistoryWindowHandle;
use crate::ports::DialogPorts;
use nerust_gui_shell::session::osd::ModalSource as _;
use nerust_gui_shell::session::osd::OsdManager;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum HostAction {
    None,
    RomLoaded,
    Exit,
}

const DEFAULT_FIT_WINDOW_WIDTH: f64 = 960.0;
const DEFAULT_FIT_WINDOW_HEIGHT: f64 = 720.0;

/// Headless `HostState`: temp settings paths, recording menu.
#[cfg(test)]
pub(crate) struct TestHost {
    pub(crate) host: HostState,
    pub(crate) menu: crate::test_menu::MenuRecorderHandle,
    pub(crate) loader: crate::test_session::FakeRomLoader,
    pub(crate) dialogs: crate::test_dialogs::FakeDialogs,
}

#[cfg(test)]
impl TestHost {
    pub(crate) fn new() -> Self {
        Self::with_pick(None)
    }

    pub(crate) fn with_pick(pick: Option<PathBuf>) -> Self {
        Self::with_loader_and_pick(crate::test_session::FakeRomLoader::succeeding(), pick)
    }

    pub(crate) fn with_loader_and_pick(
        loader: crate::test_session::FakeRomLoader,
        pick: Option<PathBuf>,
    ) -> Self {
        let support = crate::test_session::TestContext::with_loader(loader.clone());
        let paths = support.settings_paths();
        let menu = crate::test_menu::MenuRecorderHandle::default();
        let dialogs = crate::test_dialogs::FakeDialogs::with_pick(pick);
        let host = HostState::new_with_settings_paths(
            support.ctx,
            menu.clone().into_box(),
            std::sync::Arc::new(dialogs.clone()),
            Some(paths),
        )
        .expect("headless host builds on temp settings paths");
        Self {
            host,
            menu,
            loader,
            dialogs,
        }
    }
}

fn backend_capabilities() -> HostBackendCapabilities {
    HostBackendCapabilities {
        window: HostWindowCapabilities {
            remembers_window_size: true,
            supports_fullscreen_default: true,
            supports_scaling: true,
        },
        presentation: Some(BackendPresentationCapabilities {
            supports_vsync: true,
        }),
    }
}

/// Menu signature for the chrome dirty gate; `generation` fingerprints content.
/// The window title keeps its own gate, so title-only moves never touch the menu path.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct MenuSig {
    pub(super) loaded: bool,
    pub(super) paused: bool,
    pub(super) settings_open: bool,
    pub(super) language: nerust_settings_core::model::language::AppLanguage,
    pub(super) active_slot: Option<u64>,
    pub(super) slots: std::sync::Arc<[nerust_persistence::model::StateSlotSummary]>,
    pub(super) shortcuts: std::sync::Arc<[nerust_settings_core::model::input::ShortcutBinding]>,
    pub(super) generation: u64,
}

impl MenuSig {
    /// Cheap content fingerprint: FNV-1a over slot ids/timestamps and shortcut codes.
    pub(super) fn fingerprint(
        slots: &[nerust_persistence::model::StateSlotSummary],
        shortcuts: &[nerust_settings_core::model::input::ShortcutBinding],
    ) -> u64 {
        let mut hash: u64 = 0xcbf29ce484222325;
        let mut mix = |byte: u8| {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x100000001b3);
        };
        for slot in slots {
            for byte in slot.slot_id.to_le_bytes() {
                mix(byte);
            }
            if let Ok(age) = slot.saved_at.duration_since(std::time::UNIX_EPOCH) {
                for byte in age.as_secs().to_le_bytes() {
                    mix(byte);
                }
            }
            for byte in slot.emulator_version.as_bytes() {
                mix(*byte);
            }
        }
        for shortcut in shortcuts {
            for byte in format!("{:?}{:?}", shortcut.action, shortcut.key).bytes() {
                mix(byte);
            }
        }
        hash
    }
}

/// Scalar snapshot for the cheap chrome gate. One struct instead
/// of eight positional scalars at the call site.
pub(super) struct ChromeScalars<'a> {
    pub(super) loaded: bool,
    pub(super) paused: bool,
    pub(super) settings_open: bool,
    pub(super) language: nerust_settings_core::model::language::AppLanguage,
    pub(super) active_slot: Option<u64>,
    pub(super) slots_len: usize,
    pub(super) shortcuts_len: usize,
    pub(super) title: &'a str,
}

/// Chrome sync state: last pushed menu signature plus last pushed window title.
#[derive(Debug, Default)]
pub(super) struct ChromeSync {
    last_menu_sig: Option<MenuSig>,
    last_title: crate::aux::focus::Memo<String>,
}

impl ChromeSync {
    fn new() -> Self {
        Self {
            last_menu_sig: None,
            last_title: crate::aux::focus::Memo::new(),
        }
    }

    /// Cheap scalar gate: proves change without touching slices.
    /// A match falls through to the exact fingerprint/slice compare.
    pub(super) fn cheap_same(&self, scalars: &ChromeScalars) -> bool {
        self.last_menu_sig.as_ref().is_some_and(|sig| {
            sig.loaded == scalars.loaded
                && sig.paused == scalars.paused
                && sig.settings_open == scalars.settings_open
                && sig.language == scalars.language
                && sig.active_slot == scalars.active_slot
                && sig.slots.len() == scalars.slots_len
                && sig.shortcuts.len() == scalars.shortcuts_len
        }) && self
            .last_title
            .get()
            .is_some_and(|last| last == scalars.title)
    }

    /// Exact gate: generation first, then slice compare. True means
    /// the menu IPC can be skipped.
    pub(super) fn menu_same(
        &self,
        generation: u64,
        slots: &[nerust_persistence::model::StateSlotSummary],
        shortcuts: &[nerust_settings_core::model::input::ShortcutBinding],
    ) -> bool {
        self.last_menu_sig.as_ref().is_some_and(|sig| {
            sig.generation == generation
                && slots == sig.slots.as_ref()
                && shortcuts == sig.shortcuts.as_ref()
        })
    }

    pub(super) fn push_menu(&mut self, sig: MenuSig) {
        self.last_menu_sig = Some(sig);
    }

    /// Title-only gate: never touches the menu signature.
    pub(super) fn title_same(&self, title: &str) -> bool {
        self.last_title.get().is_some_and(|last| last == title)
    }

    pub(super) fn push_title(&mut self, title: String) {
        self.last_title.check(&title);
    }
}

/// Settings-window cluster: handle plus its resume bookkeeping. Single
/// owner for the settings-modal domain; the open truth is the handle.
pub(super) struct SettingsHost {
    window: Option<crate::aux::settings::window::SettingsWindowHandle>,
    resume_after: bool,
}

impl SettingsHost {
    /// Seat a newly created settings window (single writer: the open
    /// path in `HostState::open_settings_window`).
    pub(super) fn open(&mut self, handle: crate::aux::settings::window::SettingsWindowHandle) {
        self.window = Some(handle);
    }

    /// Close teardown: drop the handle and clear the resume flag, so
    /// no stale resume leaks past window close.
    pub(super) fn close_all(&mut self) {
        self.window.take();
        self.resume_after = false;
    }

    /// Whether the settings window is currently open. Single truth:
    /// no bool shadow to drift from the handle.
    pub(super) fn is_open(&self) -> bool {
        self.window.is_some()
    }

    /// Record whether closing must resume the session (test and the
    /// open path only; teardown consumes it via `take_resume_after`).
    pub(super) fn set_resume_after(&mut self, resume: bool) {
        self.resume_after = resume;
    }

    /// Consume the pending resume flag for close teardown.
    pub(super) fn take_resume_after(&mut self) -> bool {
        std::mem::take(&mut self.resume_after)
    }

    /// Take the settings window for close teardown, if one is open.
    /// Single writer alongside `open` (see `HostState::open_settings_window`).
    fn take(&mut self) -> Option<crate::aux::settings::window::SettingsWindowHandle> {
        self.window.take()
    }

    /// Borrow the settings window for event routing, if one is open.
    fn borrow_mut(&mut self) -> Option<&mut crate::aux::settings::window::SettingsWindowHandle> {
        self.window.as_mut()
    }
}

impl SettingsHost {
    fn new() -> Self {
        Self {
            window: None,
            resume_after: false,
        }
    }
}

/// OSD service: one owner for the overlay domain.
/// `commands.rs` routes through the ports below, never the fields.
pub(super) struct OsdService {
    pub(super) manager: OsdManager,
    pub(super) raster: nerust_render_osd::OsdRaster,
    /// Owns the whole OSD sync fingerprint; this host only rasterizes the plan.
    pub(super) presenter: nerust_gui_shell::session::osd::OsdPresenter,
    /// Last sync left a layer on the renderer; `Clear` needs it to know a layer exists.
    pub(super) layer_live: bool,
    /// Last applied edge strip; unchanged strips skip the renderer call.
    pub(super) last_strip: crate::aux::focus::Memo<nerust_render_traits::ReservedStrip>,
    pub(super) history_window: Option<HistoryWindowHandle>,
    /// In-flight async error dialog plus resume bookkeeping.
    pub(super) modal_flight: Option<ModalFlight>,
    pub(super) resume_after_modal: bool,
    /// Last synced history content: sequence plus language (gated via [`Memo`]).
    pub(super) history_sync_key:
        crate::aux::focus::Memo<(u64, nerust_settings_core::model::language::AppLanguage)>,
    /// Last geometry the presenter ran with: surface size plus chrome.
    pub(super) last_sync_geometry:
        crate::aux::focus::Memo<(u32, u32, nerust_gui_shell::session::osd::OsdChrome)>,
}

impl OsdService {
    /// Borrow the history window for event routing, if one is open.
    pub(super) fn history_window_mut(
        &mut self,
    ) -> Option<&mut crate::aux::osd::HistoryWindowHandle> {
        self.history_window.as_mut()
    }

    /// Close teardown: drop the history viewer, sync memo, dialog, and resume flag.
    pub(super) fn close_all(&mut self) {
        self.history_window.take();
        self.history_sync_key.clear();
        self.modal_flight.take();
        self.resume_after_modal = false;
    }

    /// Queued error batches awaiting a dialog (test read;
    /// production drains through the modal ports).
    #[cfg(test)]
    pub(super) fn modal_pending(&self) -> usize {
        self.manager.modal_pending()
    }

    /// Test seam: simulate a dialog thread that died before acking.
    #[cfg(test)]
    pub(super) fn inject_modal_flight_for_test(&mut self, flight: ModalFlight) {
        self.modal_flight = Some(flight);
    }

    /// Whether an error dialog is currently showing (test read).
    #[cfg(test)]
    pub(super) fn modal_showing(&self) -> bool {
        self.modal_flight.is_some()
    }

    /// Last synced history content key (test read).
    #[cfg(test)]
    pub(super) fn history_key(
        &self,
    ) -> Option<(u64, nerust_settings_core::model::language::AppLanguage)> {
        self.history_sync_key.get().copied()
    }

    /// Port for the modal drain: next queued error batch, if any and
    /// no dialog is showing.
    pub(super) fn take_modal_batch(&mut self) -> Option<String> {
        if self.modal_flight.is_some() {
            return None;
        }
        self.manager.take_modal_batch()
    }

    /// Port for the modal drain: close the acked batch.
    pub(super) fn modal_closed(&mut self) {
        self.manager.modal_closed();
    }

    /// Port for the modal drain: queued batch count.
    pub(super) fn modal_queue_len(&self) -> usize {
        self.manager.modal_pending()
    }

    fn new() -> Self {
        Self {
            manager: OsdManager::new(),
            raster: nerust_render_osd::OsdRaster::new(),
            presenter: nerust_gui_shell::session::osd::OsdPresenter::new(),
            layer_live: false,
            last_strip: crate::aux::focus::Memo::new(),
            history_window: None,
            modal_flight: None,
            resume_after_modal: false,
            history_sync_key: crate::aux::focus::Memo::new(),
            last_sync_geometry: crate::aux::focus::Memo::new(),
        }
    }
}

/// Debug orchestrator: owns the aux-window cluster plus the in-flight async flights.
/// `drain.rs` routes through these narrow methods, never through `host.debug` fields.
pub(super) struct DebugOrchestrator {
    windows: DebugWindows,
    drain_flight: Option<DrainFlight>,
    live_flight: Option<LiveImagesFlight>,
}

impl DebugOrchestrator {
    fn new() -> Self {
        Self {
            windows: DebugWindows::new(),
            drain_flight: None,
            live_flight: None,
        }
    }

    pub(super) fn windows(&self) -> &DebugWindows {
        &self.windows
    }

    pub(super) fn windows_mut(&mut self) -> &mut DebugWindows {
        &mut self.windows
    }

    /// Take the stored drain flight for polling.
    pub(super) fn take_drain_flight(&mut self) -> Option<DrainFlight> {
        self.drain_flight.take()
    }

    /// Store a still-pending drain flight (supersedes any older one).
    pub(super) fn store_drain_flight(&mut self, flight: DrainFlight) {
        self.drain_flight = Some(flight);
    }

    /// Take the stored live-images flight for polling.
    pub(super) fn take_live_flight(&mut self) -> Option<LiveImagesFlight> {
        self.live_flight.take()
    }

    /// Store a still-pending live-images flight.
    pub(super) fn store_live_flight(&mut self, flight: LiveImagesFlight) {
        self.live_flight = Some(flight);
    }

    pub(super) fn live_flight_pending(&self) -> bool {
        self.live_flight.is_some()
    }

    pub(super) fn close_all(&mut self) {
        self.windows.close_all();
        self.drain_flight.take();
        self.live_flight.take();
    }
}

pub(crate) struct HostState {
    /// Owned native window. All dead-handle checks flow through
    /// [`close::LiveWindow`]; no bare `Arc<TaoWindow>` escapes.
    window: Option<close::LiveWindow>,
    /// Creation failure, close idempotence, and native liveness.
    close: close::CloseGuard,
    /// Session handle. Private: `runtime` reaches session state only
    /// through the narrow accessors and drain/chrome methods below.
    session: SessionHandle,
    ctx: FrontendContext,
    app_menu: Box<dyn MenuSink>,
    dialogs: std::sync::Arc<dyn DialogPorts>,
    shell: NativeShellState,
    settings: SettingsHost,
    pending_fullscreen_sync: Option<bool>,
    /// Focus truth plus the deferred auto-pause protocol; never poked field by field.
    focus: FocusCoordinator,
    /// Aux-window cluster plus in-flight async flights.
    debug: DebugOrchestrator,
    /// OSD overlay domain. Private: `commands.rs` routes through the
    /// [`OsdService`] ports; the per-frame sync stays in `host::osd`.
    osd: OsdService,
    /// Last pushed menu signature plus window title, each with its
    /// own gate. Private: presentation syncs through [`ChromeSync`].
    chrome: ChromeSync,
    /// Linux render target below the muda menubar (`None` elsewhere).
    #[cfg(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    content_area: Option<crate::gtk_embed::ContentArea>,
}

impl HostState {
    pub(crate) fn new(
        ctx: FrontendContext,
        app_menu: Box<dyn MenuSink>,
        dialogs: std::sync::Arc<dyn DialogPorts>,
    ) -> Result<Self, String> {
        Self::new_with_settings_paths(ctx, app_menu, dialogs, None)
    }

    pub(crate) fn new_with_settings_paths(
        ctx: FrontendContext,
        app_menu: Box<dyn MenuSink>,
        dialogs: std::sync::Arc<dyn DialogPorts>,
        paths: Option<nerust_gui_runtime::settings::SettingsPaths>,
    ) -> Result<Self, String> {
        let session = match paths {
            Some(paths) => SessionHandle::new_with_settings_paths(
                backend_capabilities(),
                ctx.registry.clone(),
                ctx.audio_registry.clone(),
                paths,
            ),
            None => SessionHandle::new(
                backend_capabilities(),
                ctx.registry.clone(),
                ctx.audio_registry.clone(),
            ),
        }
        .map_err(|e| {
            log::error!("failed to create core: {e}");
            e.to_string()
        })?;
        Ok(Self {
            window: None,
            close: close::CloseGuard::new(),
            session,
            ctx,
            app_menu,
            dialogs,
            shell: NativeShellState::new(),
            settings: SettingsHost::new(),
            pending_fullscreen_sync: None,
            focus: FocusCoordinator::new(),
            debug: DebugOrchestrator::new(),
            osd: OsdService::new(),
            chrome: ChromeSync::new(),
            #[cfg(any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ))]
            content_area: None,
        })
    }

    pub(crate) fn gpu_factory(&self) -> &Rc<dyn GpuFactory> {
        &self.ctx.gpu_factory
    }

    pub(crate) fn session(&self) -> &SessionHandle {
        &self.session
    }

    pub(crate) fn session_mut(&mut self) -> &mut SessionHandle {
        &mut self.session
    }

    #[cfg(test)]
    pub(crate) fn window(&self) -> Option<&close::LiveWindow> {
        self.window.as_ref()
    }

    /// Shared native window for renderer attach paths.
    pub(crate) fn window_arc(&self) -> Option<Arc<TaoWindow>> {
        self.window.as_ref().map(|w| w.shared())
    }

    /// Narrow orchestrator access for the `runtime` drain router.
    pub(super) fn orchestrator_mut(&mut self) -> &mut DebugOrchestrator {
        &mut self.debug
    }

    /// Focus-domain entry: dispatch maps `WindowEvent::Focused` to
    /// [`FocusEvent`] and applies the returned [`FocusEffect`].
    pub(super) fn focus_event(&mut self, event: FocusEvent) -> Option<FocusEffect> {
        self.focus.on(event)
    }

    /// Record a protocol-driven pause (user pauses never set this).
    pub(super) fn note_protocol_pause(&mut self) {
        self.focus.note_protocol_pause();
    }

    /// Report an aux-window focus change. Returns whether a system-paused session should resume.
    pub(super) fn on_aux_focus(&mut self, kind: AuxKind, focused: bool) -> bool {
        self.focus.on_aux(kind, focused)
    }

    /// Main window gained focus: activate and report a pending resume.
    #[cfg(test)]
    pub(super) fn note_main_arrival(&mut self) -> bool {
        matches!(
            self.focus.on(FocusEvent::Main(true)),
            Some(FocusEffect::Resume)
        )
    }

    /// Main window lost focus: deactivate and arm the deferred pause.
    #[cfg(test)]
    pub(super) fn note_main_departure(&mut self) {
        let _ = self.focus.on(FocusEvent::Main(false));
    }

    /// Main window holds focus (for the redraw policy).
    pub(super) fn main_active(&self) -> bool {
        self.focus.main_active()
    }

    /// Settings dialog owns the screen iff its handle lives.
    /// Single truth: no bool shadow to drift from the handle.
    pub(crate) fn settings_open(&self) -> bool {
        self.settings.is_open()
    }

    /// Take the settings window for close teardown, if one is open.
    /// Single writer alongside `open_settings_window`.
    pub(super) fn take_settings_window(
        &mut self,
    ) -> Option<crate::aux::settings::window::SettingsWindowHandle> {
        self.settings.take()
    }

    /// Close-request flag raised by the settings window itself.
    /// Narrow read: dispatch checks it without borrowing the handle.
    pub(super) fn settings_close_requested(&self) -> bool {
        use crate::aux::focus::AuxWindow;
        self.settings
            .window
            .as_ref()
            .is_some_and(|handle| handle.close_requested())
    }

    /// Run `f` against the settings window, if one is open. The borrow never escapes.
    pub(super) fn with_settings_window<R>(
        &mut self,
        f: impl FnOnce(&mut crate::aux::settings::window::SettingsWindowHandle) -> R,
    ) -> Option<R> {
        self.settings.borrow_mut().map(f)
    }

    /// Render the settings window, if one is open.
    pub(super) fn render_settings_window(&mut self) {
        use crate::aux::focus::AuxWindow;
        if let Some(handle) = self.settings.borrow_mut() {
            handle.render();
        }
    }

    /// Render the history window, if one is open.
    pub(super) fn render_history_window(&mut self) {
        use crate::aux::focus::AuxWindow;
        if let Some(handle) = self.osd.history_window_mut() {
            handle.render();
        }
    }

    /// Run `f` against the debugger window, if one is open. Narrow
    /// port replacing `debug_window_mut`: the borrow never escapes.
    pub(super) fn with_debug_window<R>(
        &mut self,
        f: impl FnOnce(&mut crate::aux::debug::DebugWindowHandle) -> R,
    ) -> Option<R> {
        self.debug.windows_mut().debug_window_mut().map(f)
    }

    /// Run `f` against the PPU viewer, if one is open. Narrow port
    /// replacing `ppu_window_mut`: the borrow never escapes.
    pub(super) fn with_ppu_window<R>(
        &mut self,
        f: impl FnOnce(&mut crate::aux::ppu::PpuWindowHandle) -> R,
    ) -> Option<R> {
        self.debug.windows_mut().ppu_window_mut().map(f)
    }

    /// Render the debugger window, if one is open.
    pub(super) fn render_debug_window(&mut self) {
        use crate::aux::focus::AuxWindow;
        if let Some(handle) = self.debug.windows_mut().debug_window_mut() {
            handle.render();
        }
    }

    /// Render the PPU viewer, if one is open.
    pub(super) fn render_ppu_window(&mut self) {
        use crate::aux::focus::AuxWindow;
        if let Some(handle) = self.debug.windows_mut().ppu_window_mut() {
            handle.render();
        }
    }

    /// Close the settings window and rebuild the renderer when the render target changed.
    pub(super) fn close_settings_and_maybe_rebuild(
        &mut self,
        handle: crate::aux::settings::window::SettingsWindowHandle,
    ) -> bool {
        let plan = self.close_settings_window(handle);
        let rebuild = plan.is_some_and(|p| p.renderer_needs_rebuild);
        self.request_redraw();
        rebuild
    }

    /// Close teardown already ran. Test-only read of the idempotence
    /// guard: double-delivered close events must flush exactly once.
    #[cfg(test)]
    pub(crate) fn close_exited(&self) -> bool {
        self.close.exited()
    }
}

/// `HostState` orchestration over the [`DebugOrchestrator`].
impl HostState {
    pub(crate) fn is_debug_window(&self, window_id: WindowId) -> bool {
        self.debug.windows().is_debug_window(window_id)
    }

    pub(crate) fn is_ppu_window(&self, window_id: WindowId) -> bool {
        self.debug.windows().is_ppu_window(window_id)
    }

    pub(crate) fn is_debug_open(&self) -> bool {
        self.debug.windows().is_debug_open()
    }

    pub(crate) fn is_ppu_open(&self) -> bool {
        self.debug.windows().is_ppu_open()
    }

    /// Narrow shared bridge for PPU-viewer creation. Cloned `Arc`, never a lock.
    pub(super) fn debug_bridge_for_ppu(
        &self,
    ) -> Option<std::sync::Arc<crate::aux::bridge::DebugBridge>> {
        self.debug.windows().shared_bridge_for_ppu()
    }

    pub(crate) fn debug_window_pos(&self) -> Option<(i32, i32)> {
        self.debug.windows().debug_window_pos()
    }

    pub(crate) fn ppu_window_pos(&self) -> Option<(i32, i32)> {
        self.debug.windows().ppu_window_pos()
    }

    pub(crate) fn remember_debug_window_pos(&mut self, x: i32, y: i32) {
        self.debug.windows_mut().remember_debug_window_pos(x, y);
    }

    pub(crate) fn remember_ppu_window_pos(&mut self, x: i32, y: i32) {
        self.debug.windows_mut().remember_ppu_window_pos(x, y);
    }

    pub(crate) fn request_debug_open(&mut self) {
        self.debug.windows_mut().request_debug_open();
    }

    pub(super) fn request_ppu_open(&mut self) {
        self.debug.windows_mut().request_ppu_open();
    }

    pub(super) fn set_debug_window(&mut self, handle: crate::aux::debug::DebugWindowHandle) {
        self.debug.windows_mut().set_debug_window(handle);
    }

    pub(super) fn set_ppu_window(&mut self, handle: crate::aux::ppu::PpuWindowHandle) {
        self.debug.windows_mut().set_ppu_window(handle);
    }

    pub(super) fn take_debug_batch(
        &mut self,
    ) -> Option<(bool, Vec<nerust_gui_shell::debug::DebugAction>)> {
        self.debug.windows_mut().take_debug_batch()
    }

    pub(super) fn last_drain_paused(&self) -> Option<bool> {
        self.debug.windows().last_drain_paused()
    }

    pub(super) fn set_last_drain_paused(&mut self, paused: bool) {
        self.debug.windows_mut().set_last_drain_paused(paused);
    }

    pub(super) fn live_due(&mut self, now: std::time::Instant) -> bool {
        self.debug.windows_mut().live_due(now)
    }

    pub(super) fn next_live_tick(&self) -> Option<std::time::Instant> {
        self.debug.windows().next_live_tick()
    }

    pub(super) fn drop_debug_window(&mut self) {
        self.debug.windows_mut().drop_debug_window();
    }

    pub(super) fn drop_ppu_window(&mut self) {
        self.debug.windows_mut().drop_ppu_window();
    }

    pub(super) fn debug_close_requested(&self) -> bool {
        self.debug.windows().debug_close_requested()
    }

    pub(super) fn ppu_close_requested(&self) -> bool {
        self.debug.windows().ppu_close_requested()
    }

    /// Whether a live-images flight is still pending (control-flow
    /// backstop input).
    pub(super) fn live_flight_pending(&self) -> bool {
        self.debug.live_flight_pending()
    }
}

fn create_window_builder(
    size: TaoLogicalSize<f64>,
    title: String,
    fullscreen: bool,
) -> WindowBuilder {
    WindowBuilder::new()
        .with_title(title)
        .with_inner_size(size)
        .with_fullscreen(fullscreen_mode(fullscreen))
}

fn window_is_fullscreen(window: &TaoWindow) -> bool {
    window.fullscreen().is_some()
}

fn set_window_fullscreen(window: &TaoWindow, fullscreen: bool) {
    window.set_fullscreen(fullscreen_mode(fullscreen));
}

fn fullscreen_mode(fullscreen: bool) -> Option<Fullscreen> {
    fullscreen.then_some(Fullscreen::Borderless(None))
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct NativeFullscreenSync {
    pending_target: Option<bool>,
    persist_setting: bool,
}

fn derive_native_fullscreen_sync(
    pending_target: Option<bool>,
    fullscreen: bool,
    persisted_fullscreen: bool,
) -> NativeFullscreenSync {
    if let Some(expected) = pending_target
        && fullscreen != expected
    {
        return NativeFullscreenSync {
            pending_target: Some(expected),
            persist_setting: false,
        };
    }

    NativeFullscreenSync {
        pending_target: None,
        persist_setting: fullscreen != persisted_fullscreen,
    }
}

fn window_surface_size(size: TaoPhysicalSize<u32>) -> SurfaceSize {
    SurfaceSize::new(size.width, size.height)
}

fn logical_window_size(size: WindowSize, scaling: Option<u32>) -> TaoLogicalSize<f64> {
    let (width, height) = scaling
        .map(|scale| {
            (
                f64::from(size.width) * f64::from(scale),
                f64::from(size.height) * f64::from(scale),
            )
        })
        .unwrap_or((f64::from(size.width), f64::from(size.height)));
    TaoLogicalSize::new(width, height)
}

fn default_fit_window_size() -> TaoLogicalSize<f64> {
    TaoLogicalSize::new(DEFAULT_FIT_WINDOW_WIDTH, DEFAULT_FIT_WINDOW_HEIGHT)
}

fn logical_size_from_remembered(size: RememberedWindowSize) -> TaoLogicalSize<f64> {
    TaoLogicalSize::new(f64::from(size.width), f64::from(size.height))
}

fn element_state_to_pressed(state: ElementState) -> Option<bool> {
    Some(match state {
        ElementState::Pressed => true,
        ElementState::Released => false,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use tao::{dpi::LogicalSize as TaoLogicalSize, window::Fullscreen};

    use super::TestHost;
    use super::{NativeFullscreenSync, create_window_builder, derive_native_fullscreen_sync};

    #[test]
    fn host_accessors_expose_session_and_factory() {
        let support = TestHost::new();
        let mut host = support.host;
        let _ = host.gpu_factory();
        let _ = host.session_mut();
        assert!(!host.session().tick_metrics().loaded);
    }

    #[test]
    fn production_constructor_delegates_to_temp_paths() {
        use crate::{test_menu::MenuRecorderHandle, test_session::TestContext};
        let support = TestContext::with_loader(crate::test_session::FakeRomLoader::succeeding());
        let host = super::HostState::new(
            support.ctx,
            MenuRecorderHandle::default().into_box(),
            std::sync::Arc::new(crate::ports::NativeDialogs),
        );
        assert!(host.is_ok());
    }

    #[test]
    fn headless_host_builds_and_syncs_menu() {
        let support = TestHost::new();
        let mut host = support.host;
        assert!(!host.session().tick_metrics().loaded);
        host.sync_menu_state();
        assert_eq!(
            support.menu.updates(),
            [crate::test_menu::MenuSyncSnapshot {
                loaded: false,
                paused: false,
                settings_open: false,
            }]
        );
    }

    #[test]
    fn window_builder_requests_initial_fullscreen_when_enabled() {
        let builder =
            create_window_builder(TaoLogicalSize::new(960.0, 720.0), "nerust".into(), true);

        assert_eq!(
            builder.window.fullscreen,
            Some(Fullscreen::Borderless(None))
        );
    }

    #[test]
    fn window_builder_skips_initial_fullscreen_when_disabled() {
        let builder =
            create_window_builder(TaoLogicalSize::new(960.0, 720.0), "nerust".into(), false);

        assert_eq!(builder.window.fullscreen, None);
    }

    #[test]
    fn fullscreen_sync_ignores_transient_state_while_transition_is_pending() {
        assert_eq!(
            derive_native_fullscreen_sync(Some(true), false, true),
            NativeFullscreenSync {
                pending_target: Some(true),
                persist_setting: false,
            }
        );
    }

    #[test]
    fn fullscreen_sync_clears_pending_transition_once_window_matches_target() {
        assert_eq!(
            derive_native_fullscreen_sync(Some(true), true, true),
            NativeFullscreenSync {
                pending_target: None,
                persist_setting: false,
            }
        );
    }

    #[test]
    fn fullscreen_sync_persists_native_changes_once_no_transition_is_pending() {
        assert_eq!(
            derive_native_fullscreen_sync(None, false, true),
            NativeFullscreenSync {
                pending_target: None,
                persist_setting: true,
            }
        );
    }

    #[test]
    fn element_state_maps_to_pressed() {
        use tao::event::ElementState;

        use super::element_state_to_pressed;
        assert_eq!(element_state_to_pressed(ElementState::Pressed), Some(true));
        assert_eq!(
            element_state_to_pressed(ElementState::Released),
            Some(false)
        );
    }

    #[test]
    fn window_size_math_scales_and_falls_back() {
        use nerust_gui_shell::session::session_query::WindowSize;
        use nerust_settings_core::model::app_state::RememberedWindowSize;

        use super::{
            default_fit_window_size, fullscreen_mode, logical_size_from_remembered,
            logical_window_size, window_surface_size,
        };
        let scaled = logical_window_size(
            WindowSize {
                width: 160.0,
                height: 144.0,
            },
            Some(3),
        );
        assert_eq!((scaled.width, scaled.height), (480.0, 432.0));
        let plain = logical_window_size(
            WindowSize {
                width: 160.0,
                height: 144.0,
            },
            None,
        );
        assert_eq!((plain.width, plain.height), (160.0, 144.0));
        let remembered = logical_size_from_remembered(RememberedWindowSize {
            width: 800,
            height: 600,
        });
        assert_eq!((remembered.width, remembered.height), (800.0, 600.0));
        let fit = default_fit_window_size();
        assert_eq!((fit.width, fit.height), (960.0, 720.0));
        assert!(fullscreen_mode(true).is_some());
        assert!(fullscreen_mode(false).is_none());
        let surface = window_surface_size(tao::dpi::PhysicalSize::new(960, 720));
        assert_eq!((surface.width, surface.height), (960, 720));
    }
}

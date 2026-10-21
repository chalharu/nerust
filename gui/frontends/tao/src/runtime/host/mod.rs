use nerust_gui_runtime::{
    settings::{BackendPresentationCapabilities, HostBackendCapabilities, HostWindowCapabilities},
    shell::NativeShellState,
};
use nerust_gui_shell::{
    context::FrontendContext,
    session::{SessionHandle, osd::TickMetrics, session_query::WindowSize},
};
use nerust_render_traits::{SurfaceSize, renderer::PipelineFactory};
use nerust_settings_core::model::app_state::RememberedWindowSize;
#[cfg(test)]
use std::path::PathBuf;
use std::rc::Rc;
use tao::{
    dpi::{LogicalSize as TaoLogicalSize, PhysicalSize as TaoPhysicalSize},
    event::ElementState,
    window::{Fullscreen, Window as TaoWindow, WindowBuilder, WindowId},
};

use crate::menu::MenuSink;

use crate::aux::focus::{AuxKind, FocusCoordinator, FocusEffect, FocusEvent};
use crate::aux::registry::AuxRegistry;

mod chrome;
pub(crate) mod close;
mod commands;
pub(super) mod debug;
mod focus;
mod frame;
mod osd;
mod osd_state;
mod presentation;
mod settings;
mod window;
use chrome::ChromeSync;
use debug::DebugOrchestrator;
use frame::FrameCache;
pub(crate) use frame::FrameCtx;
use osd_state::OsdService;

pub(crate) use window::{WakeNeed, WindowCluster};

use crate::ports::DialogPorts;

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
    pub(crate) fn with_nes() -> Self {
        let loader = crate::test_session::FakeRomLoader::succeeding();
        let mut support = crate::test_session::TestContext::with_loader(loader.clone());
        let paths = support.settings_paths();
        support.ctx.registry =
            std::sync::Arc::new(nerust_gui_shell::registry::SystemRegistry::new(vec![
                nerust_nes_factory::NesFactory::boxed().into(),
            ]));
        let menu = crate::test_menu::MenuRecorderHandle::default();
        let dialogs = crate::test_dialogs::FakeDialogs::default();
        let host = HostState::new_with_settings_paths(
            support.ctx,
            menu.clone().into_box(),
            std::sync::Arc::new(dialogs.clone()),
            Some(paths),
        )
        .expect("NES host");
        Self {
            host,
            menu,
            loader,
            dialogs,
        }
    }

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

pub(crate) struct HostState {
    /// Main window cluster: handle, liveness, surface target.
    pub(super) windows: WindowCluster,
    /// Session handle. Private: `runtime` reaches session state only
    /// through the narrow accessors and drain/chrome methods below.
    session: SessionHandle,
    ctx: FrontendContext,
    app_menu: Box<dyn MenuSink>,
    dialogs: std::sync::Arc<dyn DialogPorts>,
    shell: NativeShellState,
    /// Aux-window route table: every secondary window lives here.
    pub(super) aux: AuxRegistry,
    /// Focus truth plus the deferred auto-pause protocol; never poked field by field.
    focus: FocusCoordinator,
    /// Aux-window cluster plus in-flight async flights.
    pub(super) debug: DebugOrchestrator,
    /// OSD overlay domain: layer sync plus history plus modal queue.
    osd: OsdService,
    /// Last pushed menu signature plus window title, each with its own gate.
    chrome: ChromeSync,
    /// Static chrome cache; tick metrics are never cached.
    frame_cache: FrameCache,
    pause_policy: nerust_frontend_kernel::pause::PausePolicy,
    render_retry: nerust_frontend_kernel::retry::RenderRetry,
}

impl HostState {
    #[cfg(test)]
    pub(crate) fn load_test_rom(&mut self) {
        use nerust_system_traits::factory::settings::FactorySettingsView;
        let factory = nerust_nes_factory::NesFactory::boxed();
        self.session
            .set_active_system(factory.system_id().as_ref())
            .expect("NES system activates");
        let view = FactorySettingsView {
            language: Default::default(),
            system_config: factory
                .as_system_defaults()
                .unwrap()
                .default_system_settings(),
        };
        let resolved = factory
            .resolve_load_request(&view, factory.default_load_options())
            .unwrap();
        let mut rom = vec![0; 16 + 16384 + 8192];
        rom[..6].copy_from_slice(&[b'N', b'E', b'S', 0x1a, 1, 1]);
        rom[16..19].copy_from_slice(&[0x4c, 0x00, 0x80]);
        for vector in [0x3ffa, 0x3ffc, 0x3ffe] {
            rom[16 + vector..18 + vector].copy_from_slice(&[0x00, 0x80]);
        }
        self.session
            .load_resolved(
                nerust_system_traits::factory::load::MediaObject::new(None, rom),
                resolved,
            )
            .expect("valid NROM loads");
        self.reconcile_pause();
    }

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
            windows: WindowCluster::new(),
            session,
            ctx,
            app_menu,
            dialogs,
            shell: NativeShellState::new(),
            aux: AuxRegistry::new(),
            focus: FocusCoordinator::new(),
            debug: DebugOrchestrator::new(),
            osd: OsdService::new(),
            chrome: ChromeSync::new(),
            frame_cache: FrameCache::new(),
            pause_policy: Default::default(),
            render_retry: Default::default(),
        })
    }

    /// Fresh tick per pass, sharing static chrome until its sources change.
    pub(crate) fn frame_ctx(&mut self) -> FrameCtx {
        self.frame_cache.frame_ctx(&self.session)
    }

    pub(crate) fn gpu_factory(&self) -> &Rc<dyn PipelineFactory> {
        &self.ctx.gpu_factory
    }

    pub(crate) fn session(&self) -> &SessionHandle {
        &self.session
    }

    /// Narrow live-core swap for the frame pump; the port stays inside.
    pub(crate) fn swap_frame_buffer(&mut self) {
        self.session.swap_frame_buffer();
    }

    /// Borrowed frame for the render call; `None` before the first swap.
    pub(crate) fn frame_buffer(&self) -> Option<&nerust_render_traits::FrameBuffer> {
        self.session.frame_buffer()
    }

    /// Host-origin failure detail for the notification queue.
    pub(crate) fn report_host_error(&mut self, detail: String) {
        self.session.report_external_error(
            nerust_gui_shell::session::notification::FailureOrigin::Host,
            detail,
        );
    }

    /// Clear the bounded review history.
    pub(crate) fn clear_history(&mut self) {
        self.session.clear_notification_history();
    }

    /// Cached window title for the chrome gate.
    pub(crate) fn cached_title(&mut self, tick: &TickMetrics) -> String {
        self.session.window_title_cached(tick).to_owned()
    }

    #[cfg(test)]
    pub(crate) fn window(&self) -> Option<&std::sync::Arc<tao::window::Window>> {
        self.windows.live()
    }

    /// Shared native window for renderer attach paths.
    pub(crate) fn with_main_window<R>(&self, f: impl FnOnce(&TaoWindow) -> R) -> Option<R> {
        self.windows.with_usable(f)
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

    /// Kind of the aux window behind this id, if registered.
    pub(super) fn aux_kind_of(&self, id: WindowId) -> Option<AuxKind> {
        self.aux.kind_of(crate::aux::native::native_id_of(id))
    }

    /// Pump one domain event through the matching aux entry.
    pub(super) fn aux_on_event(
        &mut self,
        kind: AuxKind,
        event: crate::aux::domain::AuxNativeEvent,
    ) -> Option<crate::aux::registry::AuxOutcome> {
        self.aux.on_event(kind, event)
    }

    /// Render the aux window of this kind, if open.
    pub(super) fn render_aux(&mut self, kind: AuxKind) -> Option<crate::aux::registry::AuxOutcome> {
        self.aux.render_kind(kind)
    }

    /// Settings window owns the screen iff registered.
    pub(crate) fn settings_open(&self) -> bool {
        self.aux.contains(AuxKind::Settings)
    }

    /// Close the settings window and rebuild the renderer when the render target changed.
    pub(super) fn close_settings_and_maybe_rebuild(&mut self) -> bool {
        let pending = self.aux.settings_take_pending();
        self.aux.remove_kind(AuxKind::Settings);
        let plan = self.close_settings_window(pending);
        let rebuild = plan.is_some_and(|p| p.renderer_needs_rebuild);
        self.request_redraw();
        rebuild
    }

    /// Close teardown already ran. Test-only read of the idempotence
    /// guard: double-delivered close events must flush exactly once.
    #[cfg(test)]
    pub(crate) fn close_exited(&self) -> bool {
        self.windows.close_exited()
    }
}

/// `HostState` orchestration over the [`DebugOrchestrator`].
impl HostState {
    pub(crate) fn is_debug_open(&self) -> bool {
        self.aux.contains(AuxKind::Debug)
    }

    pub(crate) fn is_ppu_open(&self) -> bool {
        self.aux.contains(AuxKind::Ppu)
    }

    /// Narrow shared bridge for PPU-viewer creation. Cloned `Arc`, never a lock.
    pub(super) fn debug_bridge_for_ppu(
        &self,
    ) -> Option<std::sync::Arc<crate::aux::bridge::DebugBridge>> {
        self.aux.debug_bridge()
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

    pub(super) fn last_drain_paused(&self) -> Option<bool> {
        self.debug.windows().last_drain_paused()
    }

    pub(super) fn set_last_drain_paused(&mut self, paused: bool) {
        self.debug.windows_mut().set_last_drain_paused(paused);
    }

    pub(super) fn live_due(&mut self, now: std::time::Instant) -> bool {
        self.debug.windows_mut().live_due(now)
    }

    /// Live-tick interest handle for the tick thread (parks when clear).
    pub(super) fn live_tick_armed(&self) -> std::sync::Arc<std::sync::atomic::AtomicBool> {
        self.debug.windows().live_armed()
    }

    /// Register the parked tick thread for arm wakeups.
    pub(super) fn register_tick_thread(&self, thread: std::thread::Thread) {
        self.debug.windows().register_tick_thread(thread);
    }

    pub(super) fn next_live_tick(&self) -> Option<std::time::Instant> {
        self.debug.windows().next_live_tick()
    }

    pub(super) fn drop_debug_window(&mut self) {
        self.aux.remove_kind(AuxKind::Debug);
    }

    pub(super) fn drop_ppu_window(&mut self) {
        self.aux.remove_kind(AuxKind::Ppu);
    }

    pub(super) fn drop_history_window(&mut self) {
        self.aux.remove_kind(AuxKind::History);
        self.osd.history.sync_key.clear();
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
        let tick = host.frame_ctx().tick();
        let _ = host.cached_title(&tick);
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
        let ctx = host.frame_ctx();
        host.sync_menu_state(&ctx);
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

    #[test]
    fn aux_routing_stays_total_without_windows() {
        use crate::aux::domain::NativeWindowId;
        let support = TestHost::new();
        let mut host = support.host;
        let id = unsafe { tao::window::WindowId::dummy() };
        assert!(host.aux_kind_of(id).is_none());
        let _ = NativeWindowId::dummy();
        assert!(host.with_main_window(|_| ()).is_none());
        assert!(!host.is_debug_open());
        assert!(!host.is_ppu_open());
        assert!(host.debug_bridge_for_ppu().is_none());
        assert!(host.debug_window_pos().is_none());
        assert!(host.ppu_window_pos().is_none());
        host.remember_debug_window_pos(3, 4);
        host.remember_ppu_window_pos(5, 6);
        assert_eq!(host.debug_window_pos(), Some((3, 4)));
        assert_eq!(host.ppu_window_pos(), Some((5, 6)));
        host.drop_debug_window();
        host.drop_ppu_window();
        host.drop_history_window();
        assert!(host.last_drain_paused().is_none());
        host.set_last_drain_paused(true);
        assert_eq!(host.last_drain_paused(), Some(true));
        let armed = host.live_tick_armed();
        assert!(!armed.load(std::sync::atomic::Ordering::Acquire));
        host.register_tick_thread(std::thread::current());
        assert!(host.next_live_tick().is_none());
    }

    #[test]
    fn live_gate_arms_once_then_holds() {
        let support = TestHost::new();
        let mut host = support.host;
        let now = std::time::Instant::now();
        assert!(host.live_due(now));
        assert!(host.next_live_tick().is_some());
        assert!(!host.live_due(std::time::Instant::now()));
    }

    #[test]
    fn request_flags_round_trip_without_windows() {
        let support = TestHost::new();
        let mut host = support.host;
        host.request_debug_open();
        assert!(host.take_debug_open_request());
        assert!(!host.take_debug_open_request());
        host.request_ppu_open();
        assert!(host.take_ppu_open_request());
        assert!(!host.take_ppu_open_request());
    }
}

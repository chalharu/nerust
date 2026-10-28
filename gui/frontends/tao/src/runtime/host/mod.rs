#[cfg(test)]
use nerust_frontend_viewdefs::bootstrap::{backend_capabilities, boot};
#[cfg(test)]
use std::path::PathBuf;
use tao::{
    dpi::{LogicalSize as TaoLogicalSize, PhysicalSize as TaoPhysicalSize},
    event::ElementState,
    window::{Fullscreen, Window as TaoWindow, WindowBuilder, WindowId},
};

use crate::menu::MenuSink;

use crate::aux::focus::AuxKind;
use crate::aux::registry::AuxRegistry;

mod auxiliary;
mod commands;
mod osd;
mod presentation;
mod window;

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
        let root = support.settings_root();
        support.ctx.registry =
            std::sync::Arc::new(nerust_gui_shell::registry::SystemRegistry::new(vec![
                nerust_nes_factory::NesFactory::boxed().into(),
            ]));
        let menu = crate::test_menu::MenuRecorderHandle::default();
        let dialogs = crate::test_dialogs::FakeDialogs::default();
        let boot = boot(
            support.ctx,
            backend_capabilities(true, Some(true)),
            Some(root),
        )
        .expect("NES host");
        let host = HostState::new(
            boot,
            menu.clone().into_box(),
            std::sync::Arc::new(dialogs.clone()),
        );
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
        let root = support.settings_root();
        let menu = crate::test_menu::MenuRecorderHandle::default();
        let dialogs = crate::test_dialogs::FakeDialogs::with_pick(pick);
        let boot = boot(
            support.ctx,
            backend_capabilities(true, Some(true)),
            Some(root),
        )
        .expect("headless host builds on temp settings paths");
        let host = HostState::new(
            boot,
            menu.clone().into_box(),
            std::sync::Arc::new(dialogs.clone()),
        );
        Self {
            host,
            menu,
            loader,
            dialogs,
        }
    }
}

pub(crate) struct HostState {
    /// Main window cluster: handle, liveness, surface target.
    pub(super) windows: WindowCluster,
    /// The single policy owner, injected by the composition root.
    /// Native code drives it through the trait only.
    pub(crate) core: Box<dyn wakui::app::FrontendApp>,
    app_menu: Box<dyn MenuSink>,
    dialogs: std::sync::Arc<dyn DialogPorts>,
    /// Aux-window route table: every secondary window lives here.
    pub(crate) aux: AuxRegistry,
    /// App-side debug session shared by the debugger and PPU windows.
    pub(super) debug_handle: Option<wakui::debugger::DebugHandle>,
    /// Last PPU generation pushed into the viewer.
    pub(super) ppu_generation: u64,
}

impl HostState {
    #[cfg(test)]
    pub(crate) fn load_test_rom(&mut self) {
        // Rebuild the core around a loader that installs a canned NES ROM:
        // the standard TestContext loader records paths but loads nothing.
        use nerust_frontend_viewdefs::bootstrap::{backend_capabilities, boot};
        let mut support = crate::test_session::TestContext::with_loader(
            crate::test_session::FakeRomLoader::loading_nes(),
        );
        support.ctx.registry =
            std::sync::Arc::new(nerust_gui_shell::registry::SystemRegistry::new(vec![
                nerust_nes_factory::NesFactory::boxed().into(),
            ]));
        let root = support.settings_root();
        let boot = boot(
            support.ctx,
            backend_capabilities(true, Some(true)),
            Some(root),
        )
        .expect("NES test core");
        self.core = boot.app;
        self.debug_handle = None;
        self.ppu_generation = 0;
        self.core
            .load_rom(std::path::Path::new("test.nes"))
            .unwrap();
    }

    pub(crate) fn new(
        boot: wakui::bootstrap::Boot,
        app_menu: Box<dyn MenuSink>,
        dialogs: std::sync::Arc<dyn DialogPorts>,
    ) -> Self {
        let wakui::bootstrap::Boot { app: core } = boot;
        Self {
            windows: WindowCluster::new(),
            core,
            app_menu,
            dialogs,
            aux: AuxRegistry::new(),
            debug_handle: None,
            ppu_generation: 0,
        }
    }

    #[cfg(test)]
    pub(crate) fn window(&self) -> Option<&std::sync::Arc<tao::window::Window>> {
        self.windows.live()
    }

    /// Shared native window for renderer attach paths.
    pub(crate) fn with_main_window<R>(&self, f: impl FnOnce(&TaoWindow) -> R) -> Option<R> {
        self.core
            .window_usable()
            .then(|| self.windows.with_usable(f))
            .flatten()
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

    /// Aux window of this kind still registered. Test-only read: the
    /// ordered teardown must leave no aux window behind.
    #[cfg(test)]
    pub(crate) fn aux_open(&self, kind: AuxKind) -> bool {
        self.aux.contains(kind)
    }
}

/// Native debug and history window resources.
impl HostState {
    pub(crate) fn is_debug_open(&self) -> bool {
        self.aux.contains(AuxKind::Debug)
    }

    pub(crate) fn is_ppu_open(&self) -> bool {
        self.aux.contains(AuxKind::Ppu)
    }

    pub(super) fn drop_debug_window(&mut self) {
        self.aux.remove_kind(AuxKind::Debug);
        self.maybe_close_debug_session();
    }

    pub(super) fn drop_ppu_window(&mut self) {
        self.aux.remove_kind(AuxKind::Ppu);
        self.maybe_close_debug_session();
    }

    #[cfg(test)]
    pub(super) fn drop_history_window(&mut self) {
        if let Some(id) = self.aux.route_id(AuxKind::History) {
            self.finish_widget_window(id, true);
        }
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

fn window_surface_size(size: TaoPhysicalSize<u32>) -> (u32, u32) {
    (size.width, size.height)
}

fn logical_window_size(size: (f32, f32), scaling: Option<u32>) -> TaoLogicalSize<f64> {
    let (width, height) = scaling
        .map(|scale| {
            (
                f64::from(size.0) * f64::from(scale),
                f64::from(size.1) * f64::from(scale),
            )
        })
        .unwrap_or((f64::from(size.0), f64::from(size.1)));
    TaoLogicalSize::new(width, height)
}

fn default_fit_window_size() -> TaoLogicalSize<f64> {
    TaoLogicalSize::new(DEFAULT_FIT_WINDOW_WIDTH, DEFAULT_FIT_WINDOW_HEIGHT)
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
    use super::create_window_builder;

    #[test]
    fn host_accessors_expose_session_and_factory() {
        let support = TestHost::new();
        let mut host = support.host;
        let _ = host.core.refresh_window_title();
        assert!(!host.core.tick_loaded());
    }

    #[test]
    fn production_constructor_delegates_to_temp_paths() {
        use crate::{test_menu::MenuRecorderHandle, test_session::TestContext};
        let support = TestContext::with_loader(crate::test_session::FakeRomLoader::succeeding());
        let root = support.settings_root();
        let boot = nerust_frontend_viewdefs::bootstrap::boot(
            support.ctx,
            nerust_frontend_viewdefs::bootstrap::backend_capabilities(true, Some(true)),
            Some(root),
        )
        .expect("boot on temp settings paths");
        let host = super::HostState::new(
            boot,
            MenuRecorderHandle::default().into_box(),
            std::sync::Arc::new(crate::ports::NativeDialogs),
        );
        assert!(!host.core.tick_loaded());
    }

    #[test]
    fn headless_host_builds_and_syncs_menu() {
        let support = TestHost::new();
        let mut host = support.host;
        assert!(!host.core.tick_loaded());
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
        use super::{
            default_fit_window_size, fullscreen_mode, logical_window_size, window_surface_size,
        };
        let scaled = logical_window_size((160.0, 144.0), Some(3));
        assert_eq!((scaled.width, scaled.height), (480.0, 432.0));
        let plain = logical_window_size((160.0, 144.0), None);
        assert_eq!((plain.width, plain.height), (160.0, 144.0));
        let remembered = TaoLogicalSize::new(800.0, 600.0);
        assert_eq!((remembered.width, remembered.height), (800.0, 600.0));
        let fit = default_fit_window_size();
        assert_eq!((fit.width, fit.height), (960.0, 720.0));
        assert!(fullscreen_mode(true).is_some());
        assert!(fullscreen_mode(false).is_none());
        let surface = window_surface_size(tao::dpi::PhysicalSize::new(960, 720));
        assert_eq!(surface, (960, 720));
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
        assert!(host.debug_handle().is_none());
        assert!(host.core.debug_window_pos().is_none());
        assert!(host.core.ppu_window_pos().is_none());
        host.core.remember_debug_window_pos(3, 4);
        host.core.remember_ppu_window_pos(5, 6);
        assert_eq!(host.core.debug_window_pos(), Some((3, 4)));
        assert_eq!(host.core.ppu_window_pos(), Some((5, 6)));
        host.drop_debug_window();
        host.drop_ppu_window();
        host.drop_history_window();
        let armed = host.core.live_armed();
        assert!(!armed.load(std::sync::atomic::Ordering::Acquire));
        host.core.register_tick_thread(std::thread::current());
        assert!(host.core.next_live_tick().is_none());
    }

    #[test]
    fn live_gate_arms_once_then_holds() {
        let support = TestHost::new();
        let mut host = support.host;
        let now = std::time::Instant::now();
        assert!(host.core.live_due(now));
        assert!(host.core.next_live_tick().is_some());
        assert!(!host.core.live_due(std::time::Instant::now()));
    }

    #[test]
    fn request_flags_round_trip_without_windows() {
        let support = TestHost::new();
        let mut host = support.host;
        host.core.request_debug_open();
        assert!(host.core.take_debug_open_request());
        assert!(!host.core.take_debug_open_request());
        host.core.request_ppu_open();
        assert!(host.core.take_ppu_open_request());
        assert!(!host.core.take_ppu_open_request());
    }
}

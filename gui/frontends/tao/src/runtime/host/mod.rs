use nerust_gui_runtime::settings::{
    BackendPresentationCapabilities, HostBackendCapabilities, HostWindowCapabilities,
};
use nerust_gui_shell::{context::FrontendContext, session::SessionHandle};
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
use nerust_frontend_host::application::ApplicationHost;

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
    pub(super) core: ApplicationHost,
    ctx: FrontendContext,
    app_menu: Box<dyn MenuSink>,
    dialogs: std::sync::Arc<dyn DialogPorts>,
    /// Aux-window route table: every secondary window lives here.
    pub(super) aux: AuxRegistry,
}

impl std::ops::Deref for HostState {
    type Target = ApplicationHost;
    fn deref(&self) -> &Self::Target {
        &self.core
    }
}

impl std::ops::DerefMut for HostState {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.core
    }
}

impl HostState {
    #[cfg(test)]
    pub(crate) fn load_test_rom(&mut self) {
        struct TestRomLoader;
        impl nerust_gui_shell::load::RomLoader for TestRomLoader {
            fn load_rom(
                &mut self,
                _: &std::path::Path,
                target: &mut dyn nerust_gui_shell::load::RomLoadTarget,
            ) -> Result<(), nerust_gui_shell::load::RomLoaderError> {
                use nerust_system_traits::factory::settings::FactorySettingsView;
                let factory = nerust_nes_factory::NesFactory::boxed();
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
                target
                    .set_active_system(factory.system_id().as_ref())
                    .unwrap();
                let mut rom = vec![0u8; 16 + 16384 + 8192];
                rom[..6].copy_from_slice(&[b'N', b'E', b'S', 0x1a, 1, 1]);
                rom[16..19].copy_from_slice(&[0x4c, 0x00, 0x80]);
                for vector in [0x3ffa, 0x3ffc, 0x3ffe] {
                    rom[16 + vector..18 + vector].copy_from_slice(&[0x00, 0x80]);
                }
                target.load_resolved(
                    nerust_system_traits::factory::load::MediaObject::new(None, rom),
                    resolved,
                )
            }
        }
        self.core
            .load_path(&mut TestRomLoader, std::path::Path::new("test.nes"))
            .unwrap();
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
        let factory = ctx.gpu_factory.clone();
        let mut core = ApplicationHost::new(session);
        // The factory enters the view-definition side once; frontend
        // signatures never name the pipeline type.
        core.set_gpu_factory(factory);
        Ok(Self {
            windows: WindowCluster::new(),
            core,
            ctx,
            app_menu,
            dialogs,
            aux: AuxRegistry::new(),
        })
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

    /// Close the settings window and rebuild the renderer when the render target changed.
    #[cfg(test)]
    pub(super) fn close_settings_and_maybe_rebuild(&mut self) -> bool {
        let Some(id) = self.aux.route_id(AuxKind::Settings) else {
            return false;
        };
        self.finish_widget_window(id, true)
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

    /// Narrow shared bridge for PPU-viewer creation. Cloned `Arc`, never a lock.
    pub(super) fn debug_bridge_for_ppu(
        &self,
    ) -> Option<std::sync::Arc<crate::aux::bridge::DebugBridge>> {
        self.aux.debug_bridge()
    }

    pub(super) fn drop_debug_window(&mut self) {
        self.aux.remove_kind(AuxKind::Debug);
    }

    pub(super) fn drop_ppu_window(&mut self) {
        self.aux.remove_kind(AuxKind::Ppu);
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
        let armed = host.live_armed();
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

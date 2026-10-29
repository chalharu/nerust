#[cfg(test)]
use std::path::PathBuf;
use tao::{
    dpi::{LogicalSize as TaoLogicalSize, PhysicalSize as TaoPhysicalSize},
    event::ElementState,
    window::{Fullscreen, Window as TaoWindow, WindowBuilder, WindowId},
};

use crate::menu::MenuSink;

use crate::aux::registry::AuxRegistry;

mod auxiliary;
mod commands;
mod osd;
mod presentation;
mod window;

pub(crate) use window::WindowCluster;

use crate::ports::DialogPorts;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum HostAction {
    None,
    FileLoaded,
    Exit,
}

const DEFAULT_FIT_WINDOW_WIDTH: f64 = 960.0;
const DEFAULT_FIT_WINDOW_HEIGHT: f64 = 720.0;

/// Headless `HostState` over the canned [`crate::testkit::FakeApp`].
#[cfg(test)]
pub(crate) struct TestHost {
    pub(crate) host: HostState<crate::testkit::TestAux>,
    pub(crate) menu: crate::test_menu::MenuRecorderHandle,
    pub(crate) dialogs: crate::test_dialogs::FakeDialogs,
    app: std::rc::Rc<std::cell::RefCell<crate::testkit::FakeApp>>,
}

#[cfg(test)]
impl TestHost {
    fn build(loaded: bool, failing_load: bool, pick: Option<PathBuf>) -> Self {
        use crate::testkit::{FakeApp, SharedApp};
        let app = std::rc::Rc::new(std::cell::RefCell::new(FakeApp::new()));
        app.borrow_mut().loaded = loaded;
        app.borrow_mut().failing_load = failing_load;
        let menu = crate::test_menu::MenuRecorderHandle::default();
        let dialogs = crate::test_dialogs::FakeDialogs::with_pick(pick);
        let host = HostState::new(
            wakui_core::app::Application::new(Box::new(SharedApp::new(std::rc::Rc::clone(&app)))),
            menu.clone().into_box(),
            std::sync::Arc::new(dialogs.clone()),
        );
        Self {
            host,
            menu,
            dialogs,
            app,
        }
    }

    pub(crate) fn with_nes() -> Self {
        Self::build(true, false, None)
    }

    pub(crate) fn new() -> Self {
        Self::with_pick(None)
    }

    pub(crate) fn with_pick(pick: Option<PathBuf>) -> Self {
        Self::build(false, false, pick)
    }

    pub(crate) fn with_failing_pick(pick: Option<PathBuf>) -> Self {
        Self::build(false, true, pick)
    }

    pub(crate) fn menu_ref(
        &self,
        command: crate::testkit::TestMenu,
    ) -> wakui_core::commands::CommandRef {
        self.app.borrow().commands.reference(command)
    }
}

pub(crate) struct HostState<K: Copy + Eq + std::hash::Hash + 'static> {
    /// Main window cluster: handle, liveness, surface target.
    pub(super) windows: WindowCluster,
    /// The single policy owner, injected by the composition root.
    /// Native code drives it through the trait only.
    pub(crate) core: Box<dyn wakui_core::app::FrontendApp<K>>,
    app_menu: Box<dyn MenuSink>,
    dialogs: std::sync::Arc<dyn DialogPorts>,
    /// Aux-window route table: every secondary window lives here.
    pub(crate) aux: AuxRegistry<K>,
}

impl<K: Copy + Eq + std::hash::Hash + 'static> HostState<K> {
    #[cfg(test)]
    pub(crate) fn load_test_file(&mut self) {
        self.core
            .load_file(std::path::Path::new("test.nes"))
            .unwrap();
    }

    pub(crate) fn new<A>(
        app: wakui_core::app::Application<K, A>,
        app_menu: Box<dyn MenuSink>,
        dialogs: std::sync::Arc<dyn DialogPorts>,
    ) -> Self {
        let core = app.into_app();
        Self {
            windows: WindowCluster::new(),
            core,
            app_menu,
            dialogs,
            aux: AuxRegistry::new(),
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

    /// Id of the aux window behind this native id, if registered.
    pub(super) fn aux_id_of(&self, id: WindowId) -> Option<wakui_core::aux::AuxId> {
        self.aux.id_of(crate::aux::native::native_id_of(id))
    }

    /// Pump one domain event through the matching aux entry.
    pub(super) fn aux_on_event(
        &mut self,
        id: wakui_core::aux::AuxId,
        event: crate::domain::AuxNativeEvent,
    ) -> Option<crate::aux::registry::AuxOutcome> {
        self.aux.on_event(id, event)
    }

    /// Render the aux window behind this id, if open.
    pub(super) fn render_aux(
        &mut self,
        id: wakui_core::aux::AuxId,
    ) -> Option<crate::aux::registry::AuxOutcome> {
        self.aux.render_id(id)
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
        assert!(!host.core.content_loaded());
    }

    #[test]
    fn production_constructor_delegates_to_temp_paths() {
        use crate::test_menu::MenuRecorderHandle;
        let boot =
            wakui_core::app::Application::new(Box::new(crate::testkit::SharedApp::for_test()));
        let host = super::HostState::new(
            boot,
            MenuRecorderHandle::default().into_box(),
            std::sync::Arc::new(crate::ports::NativeDialogs),
        );
        assert!(!host.core.content_loaded());
    }

    #[test]
    fn headless_host_builds_and_syncs_menu() {
        let support = TestHost::new();
        let mut host = support.host;
        assert!(!host.core.content_loaded());
        host.sync_menu_state();
        assert_eq!(
            support.menu.updates(),
            [crate::test_menu::MenuSyncSnapshot {
                loaded: false,
                paused: false,
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
        use crate::domain::NativeWindowId;
        use crate::testkit::TestAux;
        let support = TestHost::new();
        let mut host = support.host;
        let id = unsafe { tao::window::WindowId::dummy() };
        assert!(host.aux_id_of(id).is_none());
        let _ = NativeWindowId::dummy();
        assert!(host.with_main_window(|_| ()).is_none());
        assert!(host.core.polled_widget().is_none());
        assert!(host.core.window_pos(TestAux::Debug).is_none());
        assert!(host.core.window_pos(TestAux::Settings).is_none());
        host.core.remember_window_pos(TestAux::Debug, 3, 4);
        host.core.remember_window_pos(TestAux::Settings, 5, 6);
        assert_eq!(host.core.window_pos(TestAux::Debug), Some((3, 4)));
        assert_eq!(host.core.window_pos(TestAux::Settings), Some((5, 6)));
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
        use crate::testkit::TestAux;
        let support = TestHost::new();
        let mut host = support.host;
        host.core.request_open(TestAux::Debug);
        assert!(host.core.take_open_request(TestAux::Debug));
        assert!(!host.core.take_open_request(TestAux::Debug));
        host.core.request_open(TestAux::Ppu);
        assert!(host.core.take_open_request(TestAux::Ppu));
        assert!(!host.core.take_open_request(TestAux::Ppu));
        assert!(host.core.drain_open_requests().is_empty());
    }
}

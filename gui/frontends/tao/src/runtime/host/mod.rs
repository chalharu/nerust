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
    window::{Fullscreen, Window as TaoWindow, WindowBuilder},
};

use crate::menu::MenuSink;

use super::debug_windows::DebugWindows;

pub(crate) mod close;
mod commands;
mod focus;
mod osd;
use osd::ModalFlight;
mod presentation;
mod settings;
mod window;

use crate::aux::focus::FocusState;
use crate::aux::osd::HistoryWindowHandle;
use crate::ports::DialogPorts;
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
/// Lives here (not in `test_*`) so the fake modules never
/// depend back on `host` (see `crate::ports` for the seam split).
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

/// Settings-window cluster: handle plus its resume bookkeeping. Single
/// owner for the settings-modal domain; the open truth is the handle.
pub(super) struct SettingsHost {
    window: Option<crate::aux::settings::window::SettingsWindowHandle>,
    resume_after: bool,
}

impl SettingsHost {
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

/// OSD cluster: shell display state, raster, presenter, layer memo,
/// modal flight, and history viewer. One owner for the overlay domain.
pub(super) struct OsdHost {
    manager: OsdManager,
    raster: nerust_render_osd::OsdRaster,
    /// Owns the whole OSD sync fingerprint. The per-frame decision
    /// procedure lives in shell `OsdPresenter`; this host only
    /// rasterizes the resulting plan.
    presenter: nerust_gui_shell::session::osd::OsdPresenter,
    /// Last sync left a layer on the renderer. Memo of the action
    /// this host took: the pre-gate needs liveness without shell.
    layer_live: bool,
    /// Last applied edge strip. The reservation only changes with
    /// settings or surface size, so unchanged strips skip the
    /// renderer call (same gate pattern as the gtk frontend).
    last_strip: nerust_render_traits::ReservedStrip,
    history_window: Option<HistoryWindowHandle>,
    /// In-flight async error dialog plus resume bookkeeping.
    modal_flight: Option<ModalFlight>,
    resume_after_modal: bool,
    /// Last synced history content: sequence plus language. The sync
    /// runs on every loop pass while the viewer is open; unchanged
    /// content skips the full row rebuild (P1 perf gate).
    history_sync_key: Option<(u64, nerust_settings_core::model::language::AppLanguage)>,
}

impl OsdHost {
    /// Borrow the history window for event routing, if one is open.
    pub(super) fn history_window_mut(
        &mut self,
    ) -> Option<&mut crate::aux::osd::HistoryWindowHandle> {
        self.history_window.as_mut()
    }

    fn new() -> Self {
        Self {
            manager: OsdManager::new(),
            raster: nerust_render_osd::OsdRaster::new(),
            presenter: nerust_gui_shell::session::osd::OsdPresenter::new(),
            layer_live: false,
            last_strip: nerust_render_traits::ReservedStrip::default(),
            history_window: None,
            modal_flight: None,
            resume_after_modal: false,
            history_sync_key: None,
        }
    }
}

pub(crate) struct HostState {
    window: Option<Arc<TaoWindow>>,
    /// Creation failure, close idempotence, and native liveness.
    close: close::CloseGuard,
    pub(super) session: SessionHandle,
    ctx: FrontendContext,
    app_menu: Box<dyn MenuSink>,
    dialogs: std::sync::Arc<dyn DialogPorts>,
    shell: NativeShellState,
    pub(super) settings: SettingsHost,
    pending_fullscreen_sync: Option<bool>,
    /// Focus truth plus the deferred auto-pause protocol. Only
    /// reachable through the intent methods below; the burst protocol
    /// must not be poked field by field.
    focus: FocusState,
    /// In-flight async debug drain: `run_debug_drain` starts it, the
    /// event loop polls it (`poll_debug_flight`). A newer batch
    /// supersedes the stored one; orphaned replies are ignored.
    pub(super) drain_flight: Option<DrainFlight>,
    /// In-flight async live-images fetch: the live tick starts it,
    /// the event loop polls it. At most one; a newer tick supersedes.
    pub(super) live_flight: Option<LiveImagesFlight>,
    /// Aux-window state (debugger + PPU viewer). Orchestration lives
    /// here; ownership lives there.
    pub(super) debug: DebugWindows,
    pub(super) osd: OsdHost,
    /// Linux render target below the muda menubar (`None` elsewhere).
    /// When it yields no native target (Wayland, pre-layout) the
    /// toplevel fallback applies and the menu stays occluded.
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
            focus: FocusState::new(),
            drain_flight: None,
            live_flight: None,
            debug: DebugWindows::new(),
            osd: OsdHost::new(),
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

    pub(crate) fn window(&self) -> Option<&Arc<TaoWindow>> {
        self.window.as_ref()
    }

    /// Close teardown already ran. Test-only read of the idempotence
    /// guard: double-delivered close events must flush exactly once.
    /// Report an aux-window focus change. Returns whether a
    /// system-paused session should resume (see `aux_focus_step`).
    pub(super) fn note_aux_focus(&mut self, focused: bool) -> bool {
        crate::aux::focus::aux_focus_step(&mut self.focus, focused)
    }

    /// Quiet aux-window focus change: cancels a pending pause without
    /// ever reporting a resume (settings must not resume anything).
    /// See `aux_focus_step_quiet`.
    pub(super) fn note_aux_focus_quiet(&mut self, focused: bool) {
        crate::aux::focus::aux_focus_step_quiet(&mut self.focus, focused);
    }

    /// Main window gained focus: activate and report a pending resume.
    pub(super) fn note_main_arrival(&mut self) -> bool {
        self.focus.arrive_main()
    }

    /// Main window lost focus: deactivate and arm the deferred pause.
    pub(super) fn note_main_departure(&mut self) {
        self.focus.leave_main();
    }

    /// Main window holds focus (for the redraw policy).
    pub(super) fn main_active(&self) -> bool {
        self.focus.main_active()
    }

    /// Settings dialog owns the screen iff its handle lives.
    /// Single truth: no bool shadow to drift from the handle.
    pub(crate) fn settings_open(&self) -> bool {
        self.settings.window.is_some()
    }

    /// Take the settings window for close teardown, if one is open.
    /// Single writer alongside `open_settings_window`.
    pub(super) fn take_settings_window(
        &mut self,
    ) -> Option<crate::aux::settings::window::SettingsWindowHandle> {
        self.settings.take()
    }

    /// Borrow the settings window for event routing, if one is open.
    pub(super) fn settings_window_mut(
        &mut self,
    ) -> Option<&mut crate::aux::settings::window::SettingsWindowHandle> {
        self.settings.borrow_mut()
    }

    /// Borrow the history window for event routing, if one is open.
    pub(super) fn history_window_mut(
        &mut self,
    ) -> Option<&mut crate::aux::osd::HistoryWindowHandle> {
        self.osd.history_window_mut()
    }

    /// Close the settings window and rebuild the renderer when the
    /// applied settings change the render target. Single home for the
    /// Destroyed and close-requested paths.
    pub(super) fn close_settings_and_maybe_rebuild(
        &mut self,
        handle: crate::aux::settings::window::SettingsWindowHandle,
    ) -> bool {
        let plan = self.close_settings_window(handle);
        let rebuild = plan.is_some_and(|p| p.renderer_needs_rebuild);
        self.request_redraw();
        rebuild
    }

    #[cfg(test)]
    pub(crate) fn close_exited(&self) -> bool {
        self.close.exited()
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
        assert!(!host.session().loaded());
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
        assert!(!host.session().loaded());
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

//! RenderSync: window/renderer ownership and frame rendering.
//! The OSD, session, and shell are borrowed per call.

use std::{rc::Rc, sync::Arc, time::Instant};

use nerust_gui_runtime::shell::NativeShellState;
use nerust_gui_shell::session::{SessionHandle, osd::TickMetrics};
use nerust_render_traits::{
    FrameBuffer, SurfaceSize, VideoRenderProfile,
    renderer::{DEFAULT_OSD_UPLOAD_RETRIES, GpuFactory, GpuRenderer, RendererConfig},
};
use nerust_settings_core::model::{language::AppLanguage, local::NotificationSettings};
use winit::{
    dpi::LogicalSize,
    event_loop::ActiveEventLoop,
    platform::android::activity::AndroidApp,
    raw_window_handle::{HasDisplayHandle, HasWindowHandle},
    window::{Window, WindowId},
};

use super::osd::{AndroidOsd, OsdSyncCtx};

/// Per-frame input bundle, built once per pass and passed to [`RenderSync::render`].
/// Borrows the session buffer instead of cloning the `Vec` every frame.
pub(super) struct FrameInput<'session> {
    pub tick: TickMetrics,
    pub notifications: NotificationSettings,
    pub language: AppLanguage,
    pub profile: Option<VideoRenderProfile>,
    pub fb: Option<&'session FrameBuffer>,
}

/// Window + renderer pair. Everything frame-related that used to live on
/// `AndroidFrontend` now routes through here.
pub(super) struct RenderSync {
    window: Option<Arc<Window>>,
    window_id: Option<WindowId>,
    renderer: Option<Box<dyn GpuRenderer>>,
}

impl RenderSync {
    pub(super) fn new() -> Self {
        Self {
            window: None,
            window_id: None,
            renderer: None,
        }
    }

    pub(super) fn window_id_matches(&self, id: WindowId) -> bool {
        self.window_id == Some(id)
    }

    pub(super) fn has_window(&self) -> bool {
        self.window.is_some()
    }

    pub(super) fn window_size(&self) -> Option<(u32, u32)> {
        self.window.as_ref().map(|window| {
            let size = window.inner_size();
            (size.width, size.height)
        })
    }

    pub(super) fn cloned_window(&self) -> Option<Arc<Window>> {
        self.window.clone()
    }

    pub(super) fn ensure_window(
        &mut self,
        event_loop: &ActiveEventLoop,
        session: &SessionHandle,
        gpu_factory: &Rc<dyn GpuFactory>,
        osd: &mut AndroidOsd,
        shell: &mut NativeShellState,
    ) -> Result<(), String> {
        if let Some(window) = self.window.as_ref().cloned() {
            if self.renderer.is_none() {
                let size = window.inner_size();
                log::info!(
                    "ensure_window: reusing existing window {}x{} and rebuilding renderer",
                    size.width,
                    size.height
                );
                self.rebuild_renderer(window, session, gpu_factory, osd, shell);
            }
            return Ok(());
        }

        log::info!("ensure_window: creating Android window");
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title(session.window_title(&session.tick_metrics()))
                        .with_resizable(false)
                        .with_inner_size(LogicalSize::new(360.0, 640.0)),
                )
                .map_err(|error| format!("failed to create Android window: {error}"))?,
        );
        self.window_id = Some(window.id());
        let size = window.inner_size();
        log::info!(
            "ensure_window: created Android window {}x{}",
            size.width,
            size.height
        );
        self.rebuild_renderer(window.clone(), session, gpu_factory, osd, shell);
        self.window = Some(window);
        Ok(())
    }

    pub(super) fn rebuild_renderer(
        &mut self,
        window: Arc<Window>,
        session: &SessionHandle,
        gpu_factory: &Rc<dyn GpuFactory>,
        osd: &mut AndroidOsd,
        shell: &mut NativeShellState,
    ) {
        let size = window.inner_size();
        log::info!(
            "rebuild_renderer: initializing renderer for {}x{}",
            size.width,
            size.height
        );
        drop(self.renderer.take());
        let vsync = session.settings_snapshot().local.video.presentation.vsync;
        let raw_window_handle = window
            .window_handle()
            .expect("failed to get window handle")
            .as_raw();
        let raw_display_handle = window
            .display_handle()
            .expect("failed to get display handle")
            .as_raw();
        let Some(render_profile) = session.render_profile().cloned() else {
            log::warn!("rebuild_renderer: no emulation core active");
            return;
        };
        let config = RendererConfig {
            render_profile,
            vsync,
            max_osd_upload_retries: DEFAULT_OSD_UPLOAD_RETRIES,
        };
        let renderer_result = gpu_factory
            .create_renderer(&config, raw_display_handle)
            .and_then(|mut r| {
                let ws = SurfaceSize::new(size.width, size.height);
                r.attach(raw_window_handle, raw_display_handle, ws)
                    .map(|_| r)
            });
        self.renderer = match renderer_result {
            Ok(renderer) => {
                log::info!("rebuild_renderer: renderer ready");
                // Fresh renderer with empty OSD state: force re-sync next frame.
                // Covers the layer, the reserved strip, and the safe area.
                osd.reset();
                self.request_redraw(shell);
                Some(renderer)
            }
            Err(e) => {
                log::error!("failed to initialize Android renderer: {e}");
                None
            }
        };
    }

    pub(super) fn request_redraw(&mut self, shell: &mut NativeShellState) {
        shell.needs_redraw = true;
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }

    pub(super) fn release_window_resources(&mut self) {
        self.renderer = None;
        self.window = None;
        self.window_id = None;
    }

    pub(super) fn release_renderer(&mut self) {
        self.renderer = None;
    }

    pub(super) fn resize_renderer(&mut self, size: SurfaceSize) {
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.resize(size);
        }
    }

    /// Build the per-frame bundle once per pass. The buffer borrows the
    /// session; `render` fans out from it without re-locking or cloning.
    pub(super) fn build_frame_input(session: &mut SessionHandle) -> FrameInput<'_> {
        // With no loaded ROM, clear the display just before rendering.
        // Guards against a stale emuthread frame racing unload.
        if !session.tick_metrics().loaded {
            session.clear_display();
        }
        session.swap_frame_buffer();
        let tick = session.tick_metrics();
        let profile = session.render_profile().cloned();
        let notifications = session.settings_snapshot().local.notifications.clone();
        let language = session.settings_snapshot().shared.general.language;
        let fb = session.frame_buffer();
        FrameInput {
            tick,
            notifications,
            language,
            profile,
            fb,
        }
    }

    pub(super) fn render(
        &mut self,
        app: &AndroidApp,
        osd: &mut AndroidOsd,
        shell: &mut NativeShellState,
        now: Instant,
        input: &FrameInput<'_>,
    ) {
        let Some(window) = self.window.as_ref() else {
            shell.needs_redraw = false;
            return;
        };
        let Some(renderer) = self.renderer.as_mut() else {
            shell.needs_redraw = false;
            return;
        };
        let window_size = SurfaceSize::new(window.inner_size().width, window.inner_size().height);
        if renderer.size() != window_size {
            renderer.resize(window_size);
        }
        let scale_factor = window.scale_factor() as f32;
        let content = input.profile.as_ref().map(|profile| profile.physical_size);
        // UI-thread push cache: render never blocks on JNI.
        // First frame falls back to a direct query.
        let system = osd.system_insets(app, window_size, scale_factor);
        let insets = AndroidOsd::safe_insets(scale_factor, content, window_size, system);
        let settings = &input.notifications;
        let language = input.language;
        let Some(fb) = input.fb.as_ref() else {
            shell.needs_redraw = false;
            return;
        };
        let Some(renderer) = self.renderer.as_mut() else {
            shell.needs_redraw = false;
            return;
        };
        osd.sync(OsdSyncCtx {
            tick: input.tick,
            settings,
            language,
            renderer: renderer.as_mut(),
            size: window_size,
            insets,
            now,
        });
        match renderer.render(fb) {
            nerust_render_traits::renderer::RenderResult::Presented => {
                shell.on_frame_presented(input.tick.frame_counter);
                self.request_redraw(shell);
            }
            nerust_render_traits::renderer::RenderResult::Skipped => {
                shell.needs_redraw = true;
            }
            nerust_render_traits::renderer::RenderResult::Error => {
                log::warn!("render: renderer reported an error");
                shell.needs_redraw = true;
            }
        }
    }
}

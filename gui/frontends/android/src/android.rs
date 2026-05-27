mod library;
mod picker;
mod renderer;
mod settings;
mod storage;
mod surface;

use self::library::LibraryDialogResult;
use self::picker::RomPickerResult;
use self::renderer::WgpuRenderer;
use self::settings::{AndroidSettings, SettingsDialogResult};
use self::storage::AndroidStorage;
use nerust_backend_wgpu::RenderResult;
use nerust_gui_runtime::settings::HostBackendIdentity;
use nerust_gui_runtime::shell::NativeShellState;
use nerust_gui_session::commands::SessionCommand;
use nerust_gui_shell::load::{LoadRequest, MediaObject};
use nerust_gui_shell::session::SessionHandle;
use nerust_gui_shell::touch::{
    PortraitTouchOverlay, TouchFrontendAction, TouchOverlayAction, TouchPoint, TouchTarget,
    actions_for_target,
};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{Touch, TouchPhase, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::platform::android::EventLoopBuilderExtAndroid;
use winit::platform::android::activity::AndroidApp;
use winit::window::{Window, WindowId};

pub(crate) fn run(app: AndroidApp) -> Result<(), String> {
    picker::bind_app(&app);
    library::bind_app(&app);
    settings::bind_app(&app);
    let frontend_app = app.clone();
    let storage_root = app
        .internal_data_path()
        .ok_or_else(|| "Android internal data path is unavailable".to_string())?;
    let storage = AndroidStorage::open(storage_root.join("nerust"))?;
    let mut builder = EventLoop::<()>::with_user_event();
    builder.with_android_app(app);
    builder.handle_volume_keys();
    let event_loop = builder
        .build()
        .map_err(|error| format!("failed to build Android event loop: {error}"))?;
    event_loop.set_control_flow(ControlFlow::Wait);

    let mut state = AndroidFrontend::new(frontend_app, storage);
    event_loop
        .run_app(&mut state)
        .map_err(|error| format!("Android event loop failed: {error}"))
}

struct AndroidFrontend {
    app: AndroidApp,
    session: SessionHandle,
    storage: AndroidStorage,
    shell: NativeShellState,
    window: Option<Arc<Window>>,
    window_id: Option<WindowId>,
    renderer: Option<WgpuRenderer>,
    overlay: Option<PortraitTouchOverlay>,
    active_touches: HashMap<u64, TouchTarget>,
}

impl AndroidFrontend {
    fn new(app: AndroidApp, storage: AndroidStorage) -> Self {
        Self {
            app,
            session: SessionHandle::new_with_settings_manager(
                HostBackendIdentity::android_wgpu(),
                storage.settings.clone(),
            ),
            storage,
            shell: NativeShellState::new(),
            window: None,
            window_id: None,
            renderer: None,
            overlay: None,
            active_touches: HashMap::new(),
        }
    }

    fn load_from_library(&mut self, id: &str) -> Result<(), String> {
        let bytes = self
            .storage
            .rom_library
            .load_bytes(id)
            .map_err(|error| format!("failed to load ROM from library: {error}"))?
            .ok_or_else(|| format!("ROM {id} was not found in the library"))?;
        let path = self.storage.rom_library.rom_path(id);
        if let Err(error) = self
            .session
            .load(MediaObject::new(path, bytes), LoadRequest::Auto)
        {
            return Err(format!("failed to start ROM {id} from library: {error}"));
        }
        self.request_redraw();
        Ok(())
    }

    fn handle_library_result(&mut self, result: LibraryDialogResult) {
        match result {
            LibraryDialogResult::Dismissed => {}
            LibraryDialogResult::Selected(id) => {
                if let Err(error) = self.load_from_library(&id) {
                    log::error!("{error}");
                }
            }
            LibraryDialogResult::ImportRequested => {
                match picker::request_open_document(&self.app) {
                    Ok(true) => {}
                    Ok(false) => {
                        log::warn!("Android ROM picker request ignored while it is already open");
                    }
                    Err(error) => {
                        log::error!("{error}");
                    }
                }
            }
        }
    }

    fn import_rom_from_uri(&mut self, uri: &str) -> Result<(), String> {
        let bytes = picker::read_uri_bytes(&self.app, uri)?;
        let (display_name, extension) = infer_import_metadata(uri);
        let entry = self
            .storage
            .rom_library
            .import_bytes(&display_name, &extension, &bytes)
            .map_err(|error| format!("failed to import Android ROM into library: {error}"))?;
        let path = self
            .storage
            .rom_library
            .rom_path(&entry.id)
            .ok_or_else(|| {
                format!(
                    "imported Android ROM {} is missing its stored file",
                    entry.id
                )
            })?;
        if let Err(error) = self
            .session
            .load(MediaObject::new(Some(path), bytes), LoadRequest::Auto)
        {
            if let Err(remove_error) = self.storage.rom_library.remove(&entry.id) {
                log::error!(
                    "failed to roll back Android ROM import {} after load error: {remove_error}",
                    entry.id
                );
            }
            return Err(format!(
                "failed to load imported Android ROM {}: {error}",
                entry.display_name
            ));
        }
        self.request_redraw();
        Ok(())
    }

    fn handle_picker_result(&mut self, result: RomPickerResult) {
        let RomPickerResult::Selected(uri) = result else {
            return;
        };
        if let Err(error) = self.import_rom_from_uri(&uri) {
            log::error!("{error}");
        }
    }

    fn handle_settings_result(&mut self, result: SettingsDialogResult) {
        let SettingsDialogResult::Applied(raw) = result else {
            return;
        };
        let Some(android_settings) = AndroidSettings::from_choice_indices(&raw) else {
            log::error!("Android settings dialog returned an unrecognisable result: {raw:?}");
            return;
        };
        let mut next = self.session.settings_snapshot().clone();
        android_settings.apply_to_snapshot(&mut next);
        match self.session.apply_settings(next) {
            Ok(plan) => {
                if plan.renderer_rebuild_required {
                    // If Android has already dropped the surface, keep the renderer absent here;
                    // `ensure_window` will rebuild it on the next resume with the updated settings.
                    self.renderer = self
                        .window
                        .as_ref()
                        .cloned()
                        .map(|window| WgpuRenderer::new(window, &self.session));
                }
                self.request_redraw();
            }
            Err(error) => {
                log::error!("failed to apply Android settings: {error}");
            }
        }
    }

    fn ensure_window(&mut self, event_loop: &ActiveEventLoop) -> Result<(), String> {
        if self.window.is_some() {
            return Ok(());
        }

        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title(self.session.window_title())
                        .with_resizable(false)
                        .with_inner_size(LogicalSize::new(360.0, 640.0)),
                )
                .map_err(|error| format!("failed to create Android window: {error}"))?,
        );
        self.window_id = Some(window.id());
        self.renderer = Some(WgpuRenderer::new(window.clone(), &self.session));
        self.window = Some(window);
        self.rebuild_overlay();
        Ok(())
    }

    fn request_redraw(&mut self) {
        self.shell.needs_redraw = true;
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }

    fn rebuild_overlay(&mut self) {
        let Some(window) = self.window.as_ref() else {
            self.overlay = None;
            return;
        };
        let size = window.inner_size();
        self.overlay = Some(PortraitTouchOverlay::new(
            size.width as f32,
            size.height as f32,
        ));
    }

    fn render(&mut self) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        let size = window.inner_size();
        match renderer.render(
            &self.session,
            nerust_screen_wgpu::surface::SurfaceSize::new(size.width, size.height),
        ) {
            RenderResult::Presented => {
                self.shell
                    .on_frame_presented(self.session.metrics().frame_counter);
            }
            RenderResult::Skipped | RenderResult::Error => {
                self.shell.needs_redraw = true;
            }
        }
    }

    fn maybe_refresh_title(&mut self, now: Instant) {
        if self.shell.should_refresh_title(now)
            && let Some(window) = self.window.as_ref()
        {
            window.set_title(&self.session.window_title());
        }
    }

    fn apply_touch_actions(&mut self, actions: Vec<TouchOverlayAction>) {
        for action in actions {
            match action {
                TouchOverlayAction::Input(event) => {
                    let _ = self.session.apply_input_event(event);
                    self.request_redraw();
                }
                TouchOverlayAction::Session(command) => {
                    let outcome = self.session.run_command(command).unwrap_or_default();
                    if outcome.needs_redraw {
                        self.request_redraw();
                    }
                }
                TouchOverlayAction::Frontend(TouchFrontendAction::OpenLibrary) => {
                    match library::request_show_library(
                        &self.app,
                        self.storage.rom_library.entries(),
                    ) {
                        Ok(true) => {}
                        Ok(false) => {
                            log::warn!(
                                "Android ROM library dialog ignored while it is already open"
                            );
                        }
                        Err(error) => {
                            log::error!("{error}");
                        }
                    }
                }
                TouchOverlayAction::Frontend(TouchFrontendAction::OpenSettings) => {
                    let current = AndroidSettings::from_snapshot(self.session.settings_snapshot());
                    match settings::request_show_settings_dialog(&self.app, &current) {
                        Ok(true) => {}
                        Ok(false) => {
                            log::warn!("Android settings dialog ignored while it is already open");
                        }
                        Err(error) => {
                            log::error!("{error}");
                        }
                    }
                }
            }
        }
    }

    fn sync_touch_target(&mut self, touch_id: u64, next_target: Option<TouchTarget>) {
        let previous = self.active_touches.get(&touch_id).copied();
        if previous == next_target {
            return;
        }
        if let Some(previous) = previous {
            self.apply_touch_actions(actions_for_target(previous, false));
            self.active_touches.remove(&touch_id);
        }
        if let Some(next) = next_target {
            self.apply_touch_actions(actions_for_target(next, true));
            self.active_touches.insert(touch_id, next);
        }
    }

    fn handle_touch(&mut self, touch: Touch) {
        let next_target = self.overlay.as_ref().and_then(|overlay| {
            overlay.hit_test(TouchPoint {
                x: touch.location.x as f32,
                y: touch.location.y as f32,
            })
        });
        match touch.phase {
            TouchPhase::Started | TouchPhase::Moved => {
                self.sync_touch_target(touch.id, next_target);
            }
            TouchPhase::Ended | TouchPhase::Cancelled => {
                self.sync_touch_target(touch.id, None);
            }
        }
    }
}

impl ApplicationHandler for AndroidFrontend {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if let Err(error) = self.ensure_window(event_loop) {
            log::error!("{error}");
            event_loop.exit();
            return;
        }
        let _ = self.session.run_command(SessionCommand::Resume);
        self.request_redraw();
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        let _ = self.session.clear_input();
        picker::reset();
        library::reset();
        settings::reset();
        self.renderer = None;
        self.window = None;
        self.window_id = None;
        self.overlay = None;
        self.active_touches.clear();
        self.shell.needs_redraw = true;
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if self.window_id != Some(window_id) {
            return;
        }

        match event {
            WindowEvent::CloseRequested | WindowEvent::Destroyed => {
                self.session.flush_before_exit();
                event_loop.exit();
            }
            WindowEvent::Focused(false) => {
                let _ = self.session.clear_input();
            }
            WindowEvent::Resized(size) => {
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.reconfigure(nerust_screen_wgpu::surface::SurfaceSize::new(
                        size.width,
                        size.height,
                    ));
                }
                self.rebuild_overlay();
                self.request_redraw();
            }
            WindowEvent::Touch(touch) => self.handle_touch(touch),
            WindowEvent::RedrawRequested => self.render(),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(result) = library::take_result() {
            self.handle_library_result(result);
        }
        if let Some(result) = picker::take_result() {
            self.handle_picker_result(result);
        }
        if let Some(result) = settings::take_result() {
            self.handle_settings_result(result);
        }
        self.maybe_refresh_title(Instant::now());
        if let Some(window) = self.window.as_ref()
            && self
                .shell
                .wants_redraw(self.session.metrics().frame_counter)
        {
            window.request_redraw();
        }
    }
}

fn infer_import_metadata(uri: &str) -> (String, String) {
    let candidate = uri
        .rsplit('/')
        .next()
        .and_then(|segment| segment.split('?').next())
        .filter(|segment| !segment.is_empty())
        .unwrap_or("Imported ROM")
        .replace("%20", " ");
    let path = Path::new(&candidate);
    let display_name = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .unwrap_or("Imported ROM")
        .to_string();
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_string();
    (display_name, extension)
}

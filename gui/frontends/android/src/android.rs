mod library;
mod menu;
mod picker;
mod renderer;
mod settings;
mod storage;
mod surface;

use self::library::LibraryDialogResult;
use self::menu::MenuAction;
use self::picker::RomPickerResult;
use self::renderer::WgpuRenderer;
use self::settings::{AndroidSettings, SettingsDialogResult};
use self::storage::AndroidStorage;
use jni::jni_str;
use nerust_backend_wgpu::RenderResult;
use nerust_gui_runtime::settings::HostBackendIdentity;
use nerust_gui_runtime::shell::NativeShellState;
use nerust_gui_session::commands::SessionCommand;
use nerust_gui_shell::load::{LoadRequest, MediaObject};
use nerust_gui_shell::session::SessionHandle;
use nerust_gui_shell::touch::{
    PortraitTouchOverlay, TouchOverlayAction, TouchPoint, TouchTarget, actions_for_target,
};
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{Touch, TouchPhase, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::platform::android::EventLoopBuilderExtAndroid;
use winit::platform::android::activity::AndroidApp;
use winit::window::{Window, WindowId};

const FOREGROUND_RETRY_BASE_DELAY: Duration = Duration::from_millis(250);
const FOREGROUND_RETRY_MAX_DELAY: Duration = Duration::from_secs(2);
const FOREGROUND_RETRY_MAX_ATTEMPTS: u32 = 20;

pub(crate) fn register_main_activity_natives(
    env: &mut jni::Env<'_>,
) -> Result<(), jni::errors::Error> {
    let class = env.find_class(jni_str!("io/github/chalharu/nerust/MainActivity"))?;
    let methods = unsafe {
        [
            jni::NativeMethod::from_raw_parts(
                jni_str!("onFilePickerResult"),
                jni_str!("(Ljava/lang/String;)V"),
                picker::Java_io_github_chalharu_nerust_MainActivity_onFilePickerResult
                    as *mut c_void,
            ),
            jni::NativeMethod::from_raw_parts(
                jni_str!("onMenuAction"),
                jni_str!("(Ljava/lang/String;)V"),
                menu::Java_io_github_chalharu_nerust_MainActivity_onMenuAction as *mut c_void,
            ),
            jni::NativeMethod::from_raw_parts(
                jni_str!("onRomLibrarySelected"),
                jni_str!("(Ljava/lang/String;)V"),
                library::Java_io_github_chalharu_nerust_MainActivity_onRomLibrarySelected
                    as *mut c_void,
            ),
            jni::NativeMethod::from_raw_parts(
                jni_str!("onSettingsDialogResult"),
                jni_str!("(Ljava/lang/String;)V"),
                settings::Java_io_github_chalharu_nerust_MainActivity_onSettingsDialogResult
                    as *mut c_void,
            ),
        ]
    };
    unsafe { env.register_native_methods(class, &methods) }
}

pub(crate) fn run(app: AndroidApp) -> Result<(), String> {
    // Best-effort re-registration from the native thread.  The primary
    // registration happens in JNI_OnLoad (called by System.loadLibrary on the
    // main thread with the app classloader).  This fallback may fail because
    // the native thread's attached env uses the system classloader.
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr() as _) };
    if let Err(error) = vm.attach_current_thread(register_main_activity_natives) {
        log::warn!("native re-registration skipped (expected on native thread): {error:?}");
    }

    picker::bind_app(&app);
    library::bind_app(&app);
    menu::bind_app(&app);
    settings::bind_app(&app);
    let frontend_app = app.clone();
    let storage_root = app
        .internal_data_path()
        .ok_or_else(|| "Android internal data path is unavailable".to_string())?;
    log::info!(
        "android::run: opening Android storage under {}",
        storage_root.join("nerust").display()
    );
    let storage = AndroidStorage::open(storage_root.join("nerust"))?;
    let mut builder = EventLoop::<()>::with_user_event();
    builder.with_android_app(app);
    let event_loop = builder
        .build()
        .map_err(|error| format!("failed to build Android event loop: {error}"))?;
    event_loop.set_control_flow(ControlFlow::Wait);

    let mut state = AndroidFrontend::new(frontend_app, storage);
    log::info!("android::run: entering Android event loop");
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
    is_resumed: bool,
    foreground_resume_pending: bool,
    foreground_retry_attempts: u32,
    foreground_retry_at: Option<Instant>,
    last_foreground_error: Option<String>,
    lifecycle_auto_paused: bool,
    lifecycle_restore_pending: bool,
}

impl AndroidFrontend {
    fn new(app: AndroidApp, storage: AndroidStorage) -> Self {
        log::info!("AndroidFrontend::new: building frontend state");
        let mut frontend = Self {
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
            is_resumed: false,
            foreground_resume_pending: false,
            foreground_retry_attempts: 0,
            foreground_retry_at: None,
            last_foreground_error: None,
            lifecycle_auto_paused: false,
            lifecycle_restore_pending: false,
        };
        // Skip automatic last-ROM restore on cold start; restore will happen on warm resume.
        frontend.refresh_dialog_caches();
        log::info!("AndroidFrontend::new: ready");
        frontend
    }

    fn restore_last_session(&mut self) -> Result<(), String> {
        log::info!("restore_last_session: checking previous ROM");
        let Some(id) = self.storage.load_last_rom_id()? else {
            log::info!("restore_last_session: no previous ROM recorded");
            return Ok(());
        };
        log::info!("restore_last_session: last ROM id={id}");
        if self.storage.rom_library.rom_path(&id).is_none() {
            log::warn!("restore_last_session: stored ROM id={id} is missing");
            self.session.clear_hidden_lifecycle_state();
            return Ok(());
        }
        // Hidden lifecycle autosaves are only intended for warm activity
        // resumes; applying them on a fresh launch can revive stale state from
        // an older app build.
        self.load_from_library_with_autosave(&id, false)
            .map_err(|error| format!("failed to restore Android last ROM: {error}"))
    }

    /// Update the cached library entries and settings so synchronous JNI
    /// callbacks (from onMenuAction) can show up-to-date dialogs.
    fn refresh_dialog_caches(&self) {
        library::update_cached_entries(self.storage.rom_library.entries());
        let current = AndroidSettings::from_snapshot(self.session.settings_snapshot());
        settings::update_cached_settings(&current);
    }

    fn load_from_library(&mut self, id: &str) -> Result<(), String> {
        self.load_from_library_with_autosave(id, false)
    }

    fn load_from_library_with_autosave(
        &mut self,
        id: &str,
        restore_hidden_state: bool,
    ) -> Result<(), String> {
        log::info!(
            "load_from_library_with_autosave: loading id={id} restore_hidden_state={restore_hidden_state}"
        );
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
        self.finish_rom_load(id, restore_hidden_state);
        log::info!("load_from_library_with_autosave: session ready for id={id}");
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
        log::info!("import_rom_from_uri: importing URI {uri}");
        let bytes = picker::read_uri_bytes(&self.app, uri)?;
        let (display_name, extension) = picker::infer_import_metadata(&self.app, uri);
        log::info!(
            "import_rom_from_uri: read '{}' ({} bytes)",
            display_name,
            bytes.len()
        );
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
        self.finish_rom_load(&entry.id, false);
        log::info!(
            "import_rom_from_uri: imported '{}' as id={}",
            entry.display_name,
            entry.id
        );
        Ok(())
    }

    fn handle_picker_result(&mut self, result: RomPickerResult) {
        let RomPickerResult::Selected(uri) = result else {
            log::info!("handle_picker_result: ROM picker dismissed");
            return;
        };
        log::info!("handle_picker_result: picker returned URI {uri}");
        if let Err(error) = self.import_rom_from_uri(&uri) {
            log::error!("{error}");
        }
    }

    fn handle_settings_result(&mut self, result: SettingsDialogResult) {
        let SettingsDialogResult::Applied(raw) = result else {
            log::info!("handle_settings_result: settings dialog dismissed");
            return;
        };
        log::info!("handle_settings_result: applying Android settings");
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
                    if let Some(window) = self.window.as_ref().cloned() {
                        self.rebuild_renderer(window);
                    }
                }
                self.request_redraw();
                // Settings changed – refresh cached settings for sync dialogs.
                settings::update_cached_settings(&android_settings);
            }
            Err(error) => {
                log::error!("failed to apply Android settings: {error}");
            }
        }
    }

    fn run_session_command(&mut self, command: SessionCommand) {
        let outcome = self.session.run_command(command).unwrap_or_default();
        if outcome.needs_redraw {
            self.request_redraw();
        }
    }

    fn save_lifecycle_state(&mut self) {
        log::info!(
            "save_lifecycle_state: paused={} lifecycle_auto_paused={} restore_pending={}",
            self.session.paused(),
            self.lifecycle_auto_paused,
            self.lifecycle_restore_pending
        );
        if !self.lifecycle_auto_paused && !self.session.paused() {
            let _ = self.session.run_command(SessionCommand::Pause);
            self.lifecycle_auto_paused = true;
            log::info!("save_lifecycle_state: auto-paused session");
        }
        if let Err(error) = self.session.clear_input() {
            log::warn!("skipping hidden lifecycle state save because input clear failed: {error}");
            self.lifecycle_restore_pending = false;
            self.session.clear_hidden_lifecycle_state();
            self.session.flush_before_exit();
            return;
        }
        self.active_touches.clear();
        self.lifecycle_restore_pending = self.session.save_hidden_lifecycle_state();
        if !self.lifecycle_restore_pending {
            self.session.clear_hidden_lifecycle_state();
            log::info!("save_lifecycle_state: no hidden lifecycle state was produced");
        } else {
            log::info!("save_lifecycle_state: hidden lifecycle state saved");
        }
        self.session.flush_before_exit();
        log::info!("save_lifecycle_state: flushed session state");
    }

    fn release_window_resources(&mut self) {
        self.release_surface_resources();
        self.window = None;
        self.window_id = None;
    }

    fn release_surface_resources(&mut self) {
        self.renderer = None;
        self.overlay = None;
        self.active_touches.clear();
        self.shell.needs_redraw = true;
    }

    fn handle_surface_close(&mut self) {
        log::warn!("handle_surface_close: surface closed");
        self.save_lifecycle_state();
        self.release_window_resources();
        if self.is_resumed {
            self.begin_foreground_resume();
        }
    }

    fn request_library_dialog(&mut self) {
        match library::request_show_library(&self.app, self.storage.rom_library.entries()) {
            Ok(true) => {}
            Ok(false) => {
                log::warn!("Android ROM library dialog ignored while it is already open");
            }
            Err(error) => {
                log::error!("{error}");
            }
        }
    }

    fn request_settings_dialog(&mut self) {
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

    fn handle_menu_action(&mut self, action: MenuAction) {
        match action {
            MenuAction::LoadState => self.run_session_command(SessionCommand::LoadActiveSlot),
            MenuAction::OpenLibrary => self.request_library_dialog(),
            MenuAction::OpenSettings => self.request_settings_dialog(),
            MenuAction::Reset => self.run_session_command(SessionCommand::Reset),
            MenuAction::SaveState => self.run_session_command(SessionCommand::SaveActiveSlotOrNew),
            MenuAction::TogglePause => self.run_session_command(SessionCommand::TogglePause),
        }
    }

    fn ensure_window(&mut self, event_loop: &ActiveEventLoop) -> Result<(), String> {
        if let Some(window) = self.window.as_ref().cloned() {
            if self.renderer.is_none() {
                let size = window.inner_size();
                log::info!(
                    "ensure_window: reusing existing window {}x{} and rebuilding renderer",
                    size.width,
                    size.height
                );
                self.rebuild_renderer(window);
                self.rebuild_overlay();
            }
            return Ok(());
        }

        log::info!("ensure_window: creating Android window");
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
        let size = window.inner_size();
        log::info!(
            "ensure_window: created Android window {}x{}",
            size.width,
            size.height
        );
        self.rebuild_renderer(window.clone());
        self.window = Some(window);
        self.rebuild_overlay();
        Ok(())
    }

    fn rebuild_renderer(&mut self, window: Arc<Window>) {
        let size = window.inner_size();
        log::info!(
            "rebuild_renderer: initializing renderer for {}x{}",
            size.width,
            size.height
        );
        drop(self.renderer.take());
        self.renderer = match WgpuRenderer::new(window, &self.session) {
            Ok(renderer) => {
                log::info!("rebuild_renderer: renderer ready");
                Some(renderer)
            }
            Err(error) => {
                log::error!("failed to initialize Android renderer: {error}");
                None
            }
        };
    }

    fn request_redraw(&mut self) {
        self.shell.needs_redraw = true;
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }

    fn finish_rom_load(&mut self, id: &str, restore_hidden_state: bool) {
        if let Err(error) = self.storage.save_last_rom_id(id) {
            log::warn!("{error}");
        }
        if restore_hidden_state {
            log::info!("finish_rom_load: restoring hidden lifecycle state for id={id}");
            self.session.load_hidden_lifecycle_state();
        } else {
            log::info!("finish_rom_load: clearing hidden lifecycle state for id={id}");
            self.session.clear_hidden_lifecycle_state();
        }
        self.lifecycle_auto_paused = false;
        self.lifecycle_restore_pending = false;
        if let Err(error) = self.session.run_command(SessionCommand::Resume) {
            log::warn!("finish_rom_load: failed to resume session for id={id}: {error}");
        }
        self.refresh_dialog_caches();
        self.request_redraw();
    }

    fn begin_foreground_resume(&mut self) {
        log::info!(
            "begin_foreground_resume: is_resumed={} window_present={} renderer_present={} restore_pending={} auto_paused={}",
            self.is_resumed,
            self.window.is_some(),
            self.renderer.is_some(),
            self.lifecycle_restore_pending,
            self.lifecycle_auto_paused
        );
        self.foreground_resume_pending = true;
        self.foreground_retry_attempts = 0;
        self.foreground_retry_at = None;
        self.last_foreground_error = None;
    }

    fn schedule_foreground_retry(&mut self) -> bool {
        if !self.is_resumed || !self.foreground_resume_pending {
            return false;
        }
        if self.foreground_retry_attempts >= FOREGROUND_RETRY_MAX_ATTEMPTS {
            self.foreground_resume_pending = false;
            log::error!(
                "giving up after {} Android window initialization attempts",
                FOREGROUND_RETRY_MAX_ATTEMPTS
            );
            return false;
        }

        let delay = FOREGROUND_RETRY_BASE_DELAY
            .saturating_mul(1_u32 << self.foreground_retry_attempts.min(3))
            .min(FOREGROUND_RETRY_MAX_DELAY);
        self.foreground_retry_attempts += 1;
        self.foreground_retry_at = Some(Instant::now() + delay);
        log::info!(
            "schedule_foreground_retry: scheduled attempt {} in {:?}",
            self.foreground_retry_attempts,
            delay
        );
        true
    }

    fn try_resume_foreground(&mut self, event_loop: &ActiveEventLoop) {
        if !self.is_resumed || !self.foreground_resume_pending {
            return;
        }
        let attempt = self.foreground_retry_attempts + 1;
        if let Some(retry_at) = self.foreground_retry_at {
            if Instant::now() < retry_at {
                event_loop.set_control_flow(ControlFlow::WaitUntil(retry_at));
                return;
            }
            self.foreground_retry_at = None;
        }
        log::info!("try_resume_foreground: attempt {attempt}");
        match self.ensure_window(event_loop) {
            Ok(()) => {
                self.last_foreground_error = None;
                self.foreground_resume_pending = false;
                self.foreground_retry_attempts = 0;
                self.foreground_retry_at = None;
                if self.lifecycle_restore_pending {
                    log::info!(
                        "try_resume_foreground: lifecycle_restore_pending=true; attempting to load last ROM and restore hidden lifecycle state"
                    );
                    match self.storage.load_last_rom_id() {
                        Ok(Some(id)) => {
                            if self.storage.rom_library.rom_path(&id).is_none() {
                                log::warn!("try_resume_foreground: stored ROM id={id} is missing");
                                self.session.clear_hidden_lifecycle_state();
                                self.lifecycle_restore_pending = false;
                            } else {
                                match self.load_from_library_with_autosave(&id, true) {
                                    Ok(()) => {
                                        log::info!(
                                            "try_resume_foreground: loaded last ROM id={id} for lifecycle restore"
                                        );
                                        // finish_rom_load will handle resume and clearing pending flags.
                                    }
                                    Err(error) => {
                                        log::warn!(
                                            "try_resume_foreground: failed to load last ROM id={id} for lifecycle restore: {error}"
                                        );
                                        self.lifecycle_restore_pending = false;
                                        self.session.clear_hidden_lifecycle_state();
                                    }
                                }
                            }
                        }
                        Ok(None) => {
                            log::info!("try_resume_foreground: no last ROM recorded");
                            self.session.clear_hidden_lifecycle_state();
                            self.lifecycle_restore_pending = false;
                        }
                        Err(error) => {
                            log::warn!(
                                "try_resume_foreground: failed to read last ROM id: {error}"
                            );
                            self.session.clear_hidden_lifecycle_state();
                            self.lifecycle_restore_pending = false;
                        }
                    }
                }
                if self.lifecycle_auto_paused {
                    let _ = self.session.run_command(SessionCommand::Resume);
                    self.lifecycle_auto_paused = false;
                    log::info!("try_resume_foreground: resumed session after lifecycle pause");
                }
                log::info!("try_resume_foreground: attempt {attempt} succeeded");
                self.request_redraw();
            }
            Err(error) => {
                log::warn!("try_resume_foreground: attempt {attempt} failed: {error}");
                self.last_foreground_error = Some(error);
                if self.schedule_foreground_retry()
                    && let Some(retry_at) = self.foreground_retry_at
                {
                    event_loop.set_control_flow(ControlFlow::WaitUntil(retry_at));
                }
            }
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
            self.shell.needs_redraw = false;
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
            RenderResult::Skipped => {
                self.shell.needs_redraw = true;
            }
            RenderResult::Error => {
                log::warn!(
                    "render: renderer reported an error for {}x{}",
                    size.width,
                    size.height
                );
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
        log::info!("ApplicationHandler::resumed");
        self.is_resumed = true;
        self.begin_foreground_resume();
        self.try_resume_foreground(event_loop);
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        log::info!("ApplicationHandler::suspended");
        self.is_resumed = false;
        self.foreground_resume_pending = false;
        self.foreground_retry_attempts = 0;
        self.foreground_retry_at = None;
        self.last_foreground_error = None;
        self.save_lifecycle_state();
        picker::reset();
        library::reset();
        menu::reset();
        settings::reset();
        self.release_window_resources();
    }

    fn window_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if self.window_id != Some(window_id) {
            return;
        }

        match event {
            WindowEvent::CloseRequested | WindowEvent::Destroyed => {
                log::warn!("window_event: {event:?}");
                self.handle_surface_close();
            }
            WindowEvent::Focused(false) => {
                log::info!("window_event: focus lost");
                let _ = self.session.clear_input();
            }
            WindowEvent::Resized(size) => {
                log::info!("window_event: resized to {}x{}", size.width, size.height);
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

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        self.try_resume_foreground(event_loop);
        if let Some(result) = library::take_result() {
            self.handle_library_result(result);
        }
        if let Some(result) = picker::take_result() {
            self.handle_picker_result(result);
        }
        if let Some(result) = settings::take_result() {
            self.handle_settings_result(result);
        }
        for action in menu::take_actions() {
            self.handle_menu_action(action);
        }
        self.maybe_refresh_title(now);

        if let Some(window) = self.window.as_ref() {
            let metrics = self.session.metrics();
            if self.shell.wants_redraw(metrics.frame_counter) {
                window.request_redraw();
            }
            if self.shell.wants_poll(metrics.loaded, metrics.paused) {
                event_loop.set_control_flow(ControlFlow::WaitUntil(
                    now + NativeShellState::FRAME_POLL_INTERVAL,
                ));
            } else {
                event_loop.set_control_flow(ControlFlow::Wait);
            }
        } else {
            event_loop.set_control_flow(ControlFlow::Wait);
        }
    }
}

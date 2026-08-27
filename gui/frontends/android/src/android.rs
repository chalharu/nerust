mod bridge;
mod menu;
mod messages;
mod picker;
mod saf;
mod settings;
mod storage;

use std::{
    collections::{HashMap, HashSet},
    ffi::c_void,
    path::PathBuf,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

use jni::{jni_sig, jni_str};
use nerust_core_traits::{
    audio::AudioBackendRegistry,
    factory::load::MediaObject,
    touch::{TouchControl, TouchControlRole, TouchOverlayAction, TouchPoint, TouchRect},
};
use nerust_gui_runtime::{
    settings::{
        BackendPresentationCapabilities, HostBackendCapabilities, HostWindowCapabilities,
        SettingsPaths, SettingsSnapshot,
    },
    shell::NativeShellState,
};
use nerust_gui_settings::shared::StoragePolicy;
use nerust_gui_shell::{
    registry::SystemRegistry,
    session::{
        SessionError, SessionHandle,
        access::{FrontendSession, SettingsResult},
        commands::{SessionCommand, SessionCommandOutcome},
    },
};
use nerust_input_traits::{AbstractKey, AttachmentId, DigitalControlId, DigitalInputEvent};
use nerust_render_traits::{
    SurfaceSize,
    renderer::{GpuFactory, GpuRenderer, RenderResult, RendererConfig},
};
use sha2::{Digest, Sha256};
use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::{Touch, TouchPhase, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    platform::android::{EventLoopBuilderExtAndroid, activity::AndroidApp},
    raw_window_handle::{HasDisplayHandle, HasWindowHandle},
    window::{Window, WindowId},
};

use self::{
    messages::{MenuAction, RomPickerResult, SettingsDialogResult},
    settings::AndroidSettings,
    storage::{AndroidStorage, LastMediaReference},
};

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
                jni_str!("onDirectoryPickerResult"),
                jni_str!("(Ljava/lang/String;)V"),
                picker::Java_io_github_chalharu_nerust_MainActivity_onDirectoryPickerResult
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

pub(crate) fn run(
    app: AndroidApp,
    system_registry: Arc<SystemRegistry>,
    audio_registry: Arc<AudioBackendRegistry>,
    gpu_factory: Rc<dyn GpuFactory>,
) -> Result<(), String> {
    // Best-effort re-registration from the native thread.  The primary
    // registration happens in JNI_OnLoad (called by System.loadLibrary on the
    // main thread with the app classloader).  This fallback may fail because
    // the native thread's attached env uses the system classloader.
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr() as _) };
    if let Err(error) = vm.attach_current_thread(register_main_activity_natives) {
        log::warn!("native re-registration skipped (expected on native thread): {error:?}");
    }

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
    bridge::bind_event_loop(event_loop.create_proxy());
    event_loop.set_control_flow(ControlFlow::Wait);

    let mut state = AndroidFrontend::new(
        frontend_app,
        storage,
        storage_root.join("nerust"),
        system_registry,
        audio_registry,
        gpu_factory,
    );
    log::info!("android::run: entering Android event loop");
    event_loop
        .run_app(&mut state)
        .map_err(|error| format!("Android event loop failed: {error}"))
}

fn show_toast(app: &AndroidApp, message: &str) {
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr() as _) };
    let _: Result<(), jni::errors::Error> = vm.attach_current_thread(|env| {
        let activity_raw = app.activity_as_ptr() as jni::sys::jobject;
        let activity = unsafe { jni::objects::JObject::from_raw(env, activity_raw) };

        let toast_class = env.find_class(jni_str!("android/widget/Toast"))?;
        let text = env.new_string(message)?;
        let toast = env.call_static_method(
            &toast_class,
            jni_str!("makeText"),
            jni_sig!("(Landroid/content/Context;Ljava/lang/CharSequence;I)Landroid/widget/Toast;"),
            &[
                jni::objects::JValue::Object(&activity),
                jni::objects::JValue::Object(text.as_ref()),
                jni::objects::JValue::Int(0),
            ],
        )?;
        let toast_obj = toast.l()?;
        let _ = env.call_method(&toast_obj, jni_str!("show"), jni_sig!("()V"), &[]);
        Ok(())
    });
}

fn configure_controls_overlay(
    app: &AndroidApp,
    settings: &nerust_gui_settings::local::TouchOverlaySettings,
) {
    let visibility = match settings.visibility {
        nerust_gui_settings::local::TouchOverlayVisibility::Always => "always",
        nerust_gui_settings::local::TouchOverlayVisibility::Auto => "auto",
        nerust_gui_settings::local::TouchOverlayVisibility::Hidden => "hidden",
    }
    .to_string();
    let opacity = i32::from(settings.opacity_percent);
    let scale = i32::from(settings.scale_percent);
    let offset = i32::from(settings.vertical_offset_percent);
    let haptics = settings.haptics;
    let app = app.clone();
    let callback_app = app.clone();
    app.run_on_java_main_thread(Box::new(move || {
        let vm = unsafe { jni::JavaVM::from_raw(callback_app.vm_as_ptr() as _) };
        let result: Result<(), jni::errors::Error> = vm.attach_current_thread(|env| {
            let activity_raw = callback_app.activity_as_ptr() as jni::sys::jobject;
            let activity = unsafe { jni::objects::JObject::from_raw(env, activity_raw) };
            let visibility = env.new_string(&visibility)?;
            env.call_method(
                &activity,
                jni_str!("configureControlsOverlay"),
                jni_sig!("(Ljava/lang/String;IIIZ)V"),
                &[
                    jni::objects::JValue::Object(visibility.as_ref()),
                    jni::objects::JValue::Int(opacity),
                    jni::objects::JValue::Int(scale),
                    jni::objects::JValue::Int(offset),
                    jni::objects::JValue::Bool(haptics),
                ],
            )?;
            Ok(())
        });
        if let Err(error) = result {
            log::warn!("failed to configure Android controls overlay: {error:?}");
        }
    }));
}

#[derive(Debug)]
struct TouchZone {
    control: TouchControl,
    bounds: TouchRect,
}

#[derive(Debug)]
struct ProfileTouchOverlay {
    zones: Vec<TouchZone>,
}

impl ProfileTouchOverlay {
    fn new(
        width: f32,
        height: f32,
        controls: Vec<TouchControl>,
        scale_percent: u8,
        vertical_offset_percent: i8,
    ) -> Self {
        let portrait = height >= width;
        let base = width.min(height);
        let scale = f32::from(scale_percent.clamp(50, 150)) / 100.0;
        let vertical_offset = height * f32::from(vertical_offset_percent.clamp(-30, 30)) / 100.0;
        let control_top = if portrait { height * 0.54 } else { 0.0 };
        let control_height = height - control_top;
        let dpad_left = base * 0.08;
        let dpad_size = base * 0.28 * scale;
        let dpad_center_x = dpad_left + dpad_size * 0.50;
        let dpad_center_y = if portrait {
            control_top + control_height * 0.58
        } else {
            height * 0.65
        } + vertical_offset;
        let dpad_arm = dpad_size * 0.28;
        let dpad_extent = dpad_size * 0.42;
        let action_size = base * 0.14 * scale;
        let action_gap = base * 0.04;
        let action_left = if portrait {
            width * 0.64
        } else {
            width - base * 0.08 - action_size * 2.0 - action_gap
        };
        let action_top = dpad_center_y - action_size * 0.50;
        let center_width = base * 0.10 * scale;
        let center_height = base * 0.068 * scale;
        let center_gap = base * 0.03;
        let center_row_width = center_width * 2.0 + center_gap;
        let center_left = (width - center_row_width) * 0.5;
        let center_top = if portrait {
            control_top + control_height * 0.16
        } else {
            height * 0.82
        } + vertical_offset;

        let bounds_for = |role| match role {
            TouchControlRole::DpadUp => TouchRect {
                x: dpad_center_x - dpad_arm * 0.5,
                y: dpad_center_y - dpad_extent,
                width: dpad_arm,
                height: dpad_extent - dpad_arm * 0.5,
            },
            TouchControlRole::DpadDown => TouchRect {
                x: dpad_center_x - dpad_arm * 0.5,
                y: dpad_center_y + dpad_arm * 0.5,
                width: dpad_arm,
                height: dpad_extent - dpad_arm * 0.5,
            },
            TouchControlRole::DpadLeft => TouchRect {
                x: dpad_center_x - dpad_extent,
                y: dpad_center_y - dpad_arm * 0.5,
                width: dpad_extent - dpad_arm * 0.5,
                height: dpad_arm,
            },
            TouchControlRole::DpadRight => TouchRect {
                x: dpad_center_x + dpad_arm * 0.5,
                y: dpad_center_y - dpad_arm * 0.5,
                width: dpad_extent - dpad_arm * 0.5,
                height: dpad_arm,
            },
            TouchControlRole::FaceButton2 => TouchRect {
                x: action_left,
                y: action_top,
                width: action_size,
                height: action_size,
            },
            TouchControlRole::FaceButton1 => TouchRect {
                x: action_left + action_size + action_gap,
                y: action_top,
                width: action_size,
                height: action_size,
            },
            TouchControlRole::Select => TouchRect {
                x: center_left,
                y: center_top,
                width: center_width,
                height: center_height,
            },
            TouchControlRole::Start => TouchRect {
                x: center_left + center_width + center_gap,
                y: center_top,
                width: center_width,
                height: center_height,
            },
        };
        Self {
            zones: controls
                .into_iter()
                .map(|control| TouchZone {
                    bounds: bounds_for(control.role),
                    control,
                })
                .collect(),
        }
    }

    fn hit_test(&self, point: TouchPoint) -> Option<(AttachmentId, DigitalControlId)> {
        self.zones
            .iter()
            .find(|zone| zone.bounds.contains(point))
            .map(|zone| (zone.control.attachment_id, zone.control.control_id))
    }
}

struct AndroidFrontend {
    app: AndroidApp,
    session: SessionHandle,
    storage: AndroidStorage,
    shell: NativeShellState,
    window: Option<Arc<Window>>,
    window_id: Option<WindowId>,
    renderer: Option<Box<dyn GpuRenderer>>,
    gpu_factory: Rc<dyn GpuFactory>,
    overlay: Option<ProfileTouchOverlay>,
    active_touches: HashMap<u64, (AttachmentId, DigitalControlId)>,
    overlay_revision: u64,
    physical_pressed: HashSet<(i32, AbstractKey)>,
    is_resumed: bool,
    foreground_resume_pending: bool,
    foreground_retry_attempts: u32,
    foreground_retry_at: Option<Instant>,
    last_foreground_error: Option<String>,
    lifecycle_auto_paused: bool,
    lifecycle_restore_pending: bool,
    pending_storage_settings: Option<SettingsSnapshot>,
    pending_legacy_digest: Option<[u8; 32]>,
}

impl AndroidFrontend {
    fn new(
        app: AndroidApp,
        storage: AndroidStorage,
        settings_root: PathBuf,
        system_registry: Arc<SystemRegistry>,
        audio_registry: Arc<AudioBackendRegistry>,
        gpu_factory: Rc<dyn GpuFactory>,
    ) -> Self {
        log::info!("AndroidFrontend::new: building frontend state");
        let capabilities = HostBackendCapabilities {
            window: HostWindowCapabilities {
                remembers_window_size: false,
                supports_fullscreen_default: false,
                supports_scaling: false,
            },
            presentation: Some(BackendPresentationCapabilities {
                supports_vsync: true,
            }),
        };
        let settings_paths =
            SettingsPaths::new(settings_root.join("config"), settings_root.join("data"));
        let mut session = SessionHandle::new_with_settings_paths(
            capabilities,
            system_registry,
            audio_registry,
            settings_paths,
        )
        .unwrap_or_else(|e| {
            log::error!("fatal: session creation failed — settings I/O may be corrupted: {e}");
            std::process::abort();
        });
        session.set_persistence_backends(
            Box::new(saf::AndroidStorageBackend::new(app.clone())),
            Box::new(saf::AndroidStorageBackend::new(app.clone())),
            Box::new(saf::AndroidStorageBackend::new(app.clone())),
        );
        if !storage.storage_policy_migration_completed() {
            let mut next = session.settings_snapshot().clone();
            if next.shared.persistence.storage_policy == StoragePolicy::Sidecar {
                next.shared.persistence.storage_policy = StoragePolicy::AppSharedData;
                if let Err(error) = session.apply_settings(next) {
                    log::error!(
                        "failed to migrate Android persistence to app shared data: {error}"
                    );
                } else if let Err(error) = storage.complete_storage_policy_migration() {
                    log::warn!("{error}");
                }
            } else if let Err(error) = storage.complete_storage_policy_migration() {
                log::warn!("{error}");
            }
        }
        let restore_pending = storage.has_restore_pending();
        let frontend = Self {
            app,
            session,
            storage,
            shell: NativeShellState::new(),
            window: None,
            window_id: None,
            renderer: None,
            gpu_factory,
            overlay: None,
            active_touches: HashMap::new(),
            overlay_revision: 0,
            physical_pressed: HashSet::new(),
            is_resumed: false,
            foreground_resume_pending: false,
            foreground_retry_attempts: 0,
            foreground_retry_at: None,
            last_foreground_error: None,
            lifecycle_auto_paused: false,
            lifecycle_restore_pending: restore_pending,
            pending_storage_settings: None,
            pending_legacy_digest: None,
        };
        if frontend.lifecycle_restore_pending {
            log::info!(
                "AndroidFrontend::new: restore_pending flag found; will attempt lifecycle state restore at foreground resume"
            );
        }
        frontend.refresh_dialog_caches();
        log::info!("AndroidFrontend::new: ready");
        frontend
    }

    /// Update the cached library entries and settings so synchronous JNI
    /// callbacks (from onMenuAction) can show up-to-date dialogs.
    fn refresh_dialog_caches(&self) {
        let mut current = AndroidSettings::from_snapshot(
            self.session.settings_snapshot(),
            self.session.registry(),
        );
        current.prioritize_system(self.session.active_system_id());
        settings::update_cached_settings(&current);
    }

    fn load_from_library_with_autosave(
        &mut self,
        event_loop: &ActiveEventLoop,
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
        let legacy_digest: [u8; 32] = Sha256::digest(&bytes).into();
        let path = self.storage.rom_library.rom_path(id);
        let media = MediaObject::new(path, bytes);

        self.load_media(event_loop, media, None, restore_hidden_state)?;
        self.pending_legacy_digest = Some(legacy_digest);
        Ok(())
    }

    fn load_document_uri(
        &mut self,
        event_loop: &ActiveEventLoop,
        reference: LastMediaReference,
        restore_hidden_state: bool,
    ) -> Result<(), String> {
        log::info!(
            "load_document_uri: loading '{}' from {} restore_hidden_state={restore_hidden_state}",
            reference.display_name,
            reference.uri
        );
        let bytes = picker::read_uri_bytes(&self.app, &reference.uri)?;
        let media = MediaObject::from_document_uri(
            reference.uri.clone(),
            reference.display_name.clone(),
            bytes,
        );
        self.load_media(event_loop, media, Some(reference), restore_hidden_state)
    }

    fn load_media(
        &mut self,
        event_loop: &ActiveEventLoop,
        media: MediaObject,
        document_reference: Option<LastMediaReference>,
        restore_hidden_state: bool,
    ) -> Result<(), String> {
        let (factory, system_id) = {
            let f = self
                .session
                .registry()
                .detect(&media)
                .map_err(|e| format!("failed to detect ROM system: {e}"))?
                .ok_or_else(|| "unsupported ROM format".to_string())?;
            let id = f.system_id();
            (f.clone(), id)
        };

        if let Some(reference) = document_reference.as_ref()
            && reference.system_id != system_id.to_string()
        {
            return Err(format!(
                "last media system mismatch: expected {}, detected {system_id}",
                reference.system_id
            ));
        }

        self.session.clear_input();
        self.active_touches.clear();
        self.physical_pressed.clear();
        nerust_gui_shell::load::RomLoadTarget::set_active_system(
            &mut self.session,
            system_id.as_ref(),
        )
        .map_err(|e| format!("failed to activate system {system_id}: {e}"))?;

        let options = self
            .session
            .default_load_options()
            .ok_or_else(|| "no active system".to_string())?;
        let view = nerust_settings_core::factory::settings_view(
            self.session.settings_snapshot(),
            system_id.as_ref(),
        );
        let resolved = factory
            .resolve_load_request(&view, options)
            .map_err(|e| format!("failed to resolve ROM load request: {e}"))?;

        if let Err(error) = self.session.load_resolved(media, resolved) {
            return Err(format!("failed to start ROM: {error}"));
        }
        let restore_document_on_restart = document_reference.as_ref().is_some_and(|reference| {
            if let Err(error) = self.storage.save_last_media_reference(reference) {
                log::warn!("{error}");
                false
            } else {
                true
            }
        });
        self.finish_rom_load(event_loop, restore_hidden_state);
        if restore_document_on_restart {
            self.storage.touch_restore_pending();
        }
        notify_rom_loaded(&self.app, &system_id.to_string());
        log::info!("load_media: session ready for system={system_id}");
        Ok(())
    }

    fn open_rom_from_uri(&mut self, event_loop: &ActiveEventLoop, uri: &str) -> Result<(), String> {
        log::info!("open_rom_from_uri: opening URI {uri}");
        let (display_name, extension) = picker::infer_import_metadata(&self.app, uri);
        let file_name = if extension.is_empty() {
            display_name
        } else {
            format!("{display_name}.{extension}")
        };
        let bytes = picker::read_uri_bytes(&self.app, uri)?;
        if let Some(expected) = self.pending_legacy_digest
            && <[u8; 32]>::from(Sha256::digest(&bytes)) != expected
        {
            return Err("selected ROM does not match the legacy library entry".to_string());
        }
        let media = MediaObject::from_document_uri(uri, &file_name, bytes);
        let detected_system = self
            .session
            .registry()
            .detect(&media)
            .map_err(|error| format!("failed to detect ROM system: {error}"))?
            .ok_or_else(|| "unsupported ROM format".to_string())?
            .system_id()
            .to_string();
        let reference = LastMediaReference::new(uri.to_string(), file_name, detected_system);
        self.load_media(event_loop, media, Some(reference), false)?;
        self.pending_legacy_digest = None;
        Ok(())
    }

    fn handle_picker_result(&mut self, event_loop: &ActiveEventLoop, result: RomPickerResult) {
        match result {
            RomPickerResult::Selected(uri) => {
                log::info!("handle_picker_result: picker returned URI {uri}");
                if let Err(error) = self.open_rom_from_uri(event_loop, &uri) {
                    log::error!("{error}");
                    show_toast(&self.app, &error);
                }
            }
            RomPickerResult::TreeSelected(uri) => {
                let Some(mut next) = self.pending_storage_settings.take() else {
                    log::warn!("ignoring unexpected Android directory picker result");
                    return;
                };
                next.shared.persistence.storage_document_tree_uri = Some(uri);
                match self.apply_settings(next) {
                    Ok(_) => self.request_redraw(),
                    Err(error) => {
                        log::error!("failed to apply Android storage directory: {error}");
                        show_toast(&self.app, "Selected directory could not be used");
                    }
                }
            }
            RomPickerResult::Cancelled => {
                self.pending_storage_settings = None;
                log::info!("handle_picker_result: picker dismissed");
            }
        }
    }

    fn handle_settings_result(&mut self, result: SettingsDialogResult) {
        let SettingsDialogResult::Applied(values) = result else {
            log::info!("handle_settings_result: settings dialog dismissed");
            return;
        };
        log::info!("handle_settings_result: applying Android settings");
        let mut current = AndroidSettings::from_snapshot(
            self.session.settings_snapshot(),
            self.session.registry(),
        );
        current.prioritize_system(self.session.active_system_id());
        let previous_storage_policy = current.storage_policy;
        let Some(android_settings) = AndroidSettings::from_keyed_indices(&values, &current) else {
            log::error!("Android settings dialog returned invalid keyed values");
            return;
        };
        let mut next = self.session.settings_snapshot().clone();
        if let Err(error) = android_settings.apply_to_snapshot(&mut next, self.session.registry()) {
            log::error!("failed to update Android system settings: {error}");
            return;
        }
        if android_settings.storage_policy != StoragePolicy::AppSharedData
            && (android_settings.storage_policy != previous_storage_policy
                || next.shared.persistence.storage_document_tree_uri.is_none())
        {
            self.pending_storage_settings = Some(next);
            match picker::request_open_document_tree(&self.app) {
                Ok(true) => {}
                Ok(false) => {
                    self.pending_storage_settings = None;
                    log::warn!("Android directory picker is already open");
                }
                Err(error) => {
                    self.pending_storage_settings = None;
                    log::error!("{error}");
                }
            }
            return;
        }
        if android_settings.storage_policy == StoragePolicy::AppSharedData {
            next.shared.persistence.storage_document_tree_uri = None;
        }
        match self.apply_settings(next) {
            Ok(result) => {
                if result.renderer_needs_rebuild {
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

    fn save_lifecycle_state(&mut self) {
        log::info!(
            "save_lifecycle_state: paused={} lifecycle_auto_paused={} restore_pending={}",
            self.session.paused(),
            self.lifecycle_auto_paused,
            self.lifecycle_restore_pending
        );
        if !self.lifecycle_auto_paused && !self.session.paused() {
            self.pause();
            self.lifecycle_auto_paused = true;
            log::info!("save_lifecycle_state: auto-paused session");
        }
        self.session.clear_input();
        self.active_touches.clear();
        self.physical_pressed.clear();
        self.lifecycle_restore_pending = self.session.save_hidden_lifecycle_state();
        if !self.lifecycle_restore_pending {
            self.session.clear_hidden_lifecycle_state();
            self.storage.clear_restore_pending();
            log::info!("save_lifecycle_state: no hidden lifecycle state was produced");
        } else {
            self.storage.touch_restore_pending();
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
        self.physical_pressed.clear();
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

    fn request_open_rom(&mut self) {
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

    fn request_settings_dialog(&mut self) {
        let mut current = AndroidSettings::from_snapshot(
            self.session.settings_snapshot(),
            self.session.registry(),
        );
        current.prioritize_system(self.session.active_system_id());
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
        log::info!("AndroidFrontend::handle_menu_action: {:?}", action);
        match action {
            MenuAction::ControllerInput {
                device_id,
                key,
                pressed,
            } => {
                self.apply_physical_controller_input(device_id, key, pressed);
            }
            MenuAction::Exit => {
                if self.session.loaded() {
                    self.session.clear_hidden_lifecycle_state();
                    self.storage.clear_restore_pending();
                    if let Err(error) = self.session.unload() {
                        log::warn!("failed to unload session on exit: {error}");
                    }
                }
                self.session.flush_before_exit();
                // Finish the activity and kill the process so the next launch
                // starts with a clean slate (swipe-kill semantics).
                let vm = unsafe { jni::JavaVM::from_raw(self.app.vm_as_ptr() as _) };
                let _: Result<(), jni::errors::Error> = vm.attach_current_thread(|env| {
                    let activity_raw = self.app.activity_as_ptr() as jni::sys::jobject;
                    let activity = unsafe { jni::objects::JObject::from_raw(env, activity_raw) };
                    let _ = env.call_method(&activity, jni_str!("finish"), jni_sig!("()V"), &[]);
                    let system = env.find_class(jni_str!("java/lang/System"))?;
                    let _ = env.call_static_method(
                        &system,
                        jni_str!("exit"),
                        jni_sig!("(I)V"),
                        &[jni::objects::JValue::Int(0)],
                    );
                    Ok(())
                });
            }
            MenuAction::Unload => {
                if self.session.loaded() {
                    self.session.clear_hidden_lifecycle_state();
                    self.storage.clear_restore_pending();
                    let _ = self.session.unload();
                }
                self.session.clear_display();
                self.request_redraw();
            }
            MenuAction::LoadState => {
                if !self.load_active_slot() {
                    show_toast(&self.app, "No save state to load");
                }
            }
            MenuAction::OpenRom => self.request_open_rom(),
            MenuAction::OpenSettings => self.request_settings_dialog(),
            MenuAction::Reset => self.reset(),
            MenuAction::SaveState => self.save_active_slot(),
            MenuAction::TogglePause => self.toggle_pause(),
        }
    }

    fn apply_physical_controller_input(&mut self, device_id: i32, key: AbstractKey, pressed: bool) {
        let changed = if pressed {
            self.physical_pressed.insert((device_id, key))
        } else {
            self.physical_pressed.remove(&(device_id, key))
        };
        if !changed {
            return;
        }
        let effective_pressed = self
            .physical_pressed
            .iter()
            .any(|(_, pressed_key)| *pressed_key == key);
        let role = match key {
            AbstractKey::Button1 => TouchControlRole::FaceButton1,
            AbstractKey::Button2 => TouchControlRole::FaceButton2,
            AbstractKey::Start => TouchControlRole::Start,
            AbstractKey::Select => TouchControlRole::Select,
            AbstractKey::DpadUp => TouchControlRole::DpadUp,
            AbstractKey::DpadDown => TouchControlRole::DpadDown,
            AbstractKey::DpadLeft => TouchControlRole::DpadLeft,
            AbstractKey::DpadRight => TouchControlRole::DpadRight,
            _ => return,
        };
        let model = self.session.touch_overlay_model(self.overlay_revision);
        let Some(control) = model
            .controls
            .into_iter()
            .find(|control| control.role == role)
        else {
            return;
        };
        let event = if effective_pressed {
            DigitalInputEvent::pressed(control.attachment_id, control.control_id)
        } else {
            DigitalInputEvent::released(control.attachment_id, control.control_id)
        };
        self.session.apply_input_event(event);
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
        let vsync = self
            .session
            .settings_snapshot()
            .local
            .video
            .presentation
            .vsync;
        let raw_window_handle = window
            .window_handle()
            .expect("failed to get window handle")
            .as_raw();
        let raw_display_handle = window
            .display_handle()
            .expect("failed to get display handle")
            .as_raw();
        let Some(render_profile) = self.session.render_profile().cloned() else {
            log::warn!("rebuild_renderer: no emulation core active");
            return;
        };
        let config = RendererConfig {
            render_profile,
            vsync,
        };
        let renderer_result = self
            .gpu_factory
            .create_renderer(&config, raw_display_handle)
            .and_then(|mut r| {
                let ws = SurfaceSize::new(size.width, size.height);
                r.attach(raw_window_handle, raw_display_handle, ws)
                    .map(|_| r)
            });
        self.renderer = match renderer_result {
            Ok(renderer) => {
                log::info!("rebuild_renderer: renderer ready");
                Some(renderer)
            }
            Err(e) => {
                log::error!("failed to initialize Android renderer: {e}");
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

    fn finish_rom_load(&mut self, event_loop: &ActiveEventLoop, restore_hidden_state: bool) {
        if restore_hidden_state {
            log::info!("finish_rom_load: restoring hidden lifecycle state");
            self.session.load_hidden_lifecycle_state();
        } else {
            log::info!("finish_rom_load: clearing hidden lifecycle state");
            self.session.clear_hidden_lifecycle_state();
        }
        self.lifecycle_auto_paused = false;
        self.lifecycle_restore_pending = false;
        self.storage.clear_restore_pending();
        self.resume();

        // Ensure a window and renderer exist immediately when resuming so that
        // request_redraw() takes effect without requiring a user tap.
        if self.window.is_none() && self.is_resumed {
            match self.ensure_window(event_loop) {
                Ok(()) => {
                    log::info!("finish_rom_load: ensured window/renderer after ROM load");
                }
                Err(error) => {
                    log::warn!(
                        "finish_rom_load: ensure_window failed: {error}; scheduling foreground resume"
                    );
                    self.begin_foreground_resume();
                }
            }
        } else if let Some(window) = self.window.as_ref().cloned() {
            // Rebuild renderer to ensure the underlying surface is fresh; dialogs or
            // system UI may have invalidated the previous surface.
            log::info!("finish_rom_load: rebuilding renderer to ensure visible surface");
            self.rebuild_renderer(window);
            self.rebuild_overlay();
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
                    match self.storage.load_last_media_reference() {
                        Ok(Some(reference)) => {
                            match self.load_document_uri(event_loop, reference, true) {
                                Ok(()) => {
                                    log::info!(
                                        "try_resume_foreground: loaded last document URI for lifecycle restore"
                                    );
                                }
                                Err(error) => {
                                    log::warn!(
                                        "try_resume_foreground: failed to load last document URI for lifecycle restore: {error}"
                                    );
                                    show_toast(
                                        &self.app,
                                        "Previous ROM is unavailable; open it again",
                                    );
                                    log::info!(
                                        "try_resume_foreground: preserving hidden state until the ROM URI is reconnected"
                                    );
                                }
                            }
                        }
                        Ok(None) => match self.storage.load_last_rom_id() {
                            Ok(Some(id)) => {
                                if self.storage.rom_library.rom_path(&id).is_none() {
                                    log::warn!(
                                        "try_resume_foreground: stored ROM id={id} is missing"
                                    );
                                    self.session.clear_hidden_lifecycle_state();
                                    self.storage.clear_restore_pending();
                                    self.lifecycle_restore_pending = false;
                                } else {
                                    match self
                                        .load_from_library_with_autosave(event_loop, &id, true)
                                    {
                                        Ok(()) => {
                                            log::info!(
                                                "try_resume_foreground: loaded last ROM id={id} for lifecycle restore"
                                            );
                                            show_toast(
                                                &self.app,
                                                "Legacy ROM restored; use Open ROM to reconnect its document",
                                            );
                                            // finish_rom_load will handle resume and clearing pending flags.
                                        }
                                        Err(error) => {
                                            log::warn!(
                                                "try_resume_foreground: failed to load last ROM id={id} for lifecycle restore: {error}"
                                            );
                                            self.session.clear_hidden_lifecycle_state();
                                            self.storage.clear_restore_pending();
                                            self.lifecycle_restore_pending = false;
                                        }
                                    }
                                }
                            }
                            Ok(None) => {
                                log::info!("try_resume_foreground: no last ROM recorded");
                                self.session.clear_hidden_lifecycle_state();
                                self.storage.clear_restore_pending();
                                self.lifecycle_restore_pending = false;
                            }
                            Err(error) => {
                                log::warn!(
                                    "try_resume_foreground: failed to read last ROM id: {error}"
                                );
                                self.session.clear_hidden_lifecycle_state();
                                self.storage.clear_restore_pending();
                                self.lifecycle_restore_pending = false;
                            }
                        },
                        Err(error) => {
                            log::warn!(
                                "try_resume_foreground: failed to read last media reference: {error}"
                            );
                            show_toast(
                                &self.app,
                                "Previous ROM reference is invalid; open it again",
                            );
                            self.session.clear_hidden_lifecycle_state();
                            self.storage.clear_restore_pending();
                            self.lifecycle_restore_pending = false;
                        }
                    }
                }
                if self.lifecycle_auto_paused {
                    self.resume();
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
        let overlay_settings = &self.session.settings_snapshot().local.touch_overlay;
        self.overlay_revision = self.overlay_revision.wrapping_add(1);
        let model = self.session.touch_overlay_model(self.overlay_revision);
        self.overlay = if overlay_settings.visibility
            == nerust_gui_settings::local::TouchOverlayVisibility::Hidden
        {
            None
        } else {
            Some(ProfileTouchOverlay::new(
                size.width as f32,
                size.height as f32,
                model.controls,
                overlay_settings.scale_percent,
                overlay_settings.vertical_offset_percent,
            ))
        };
        configure_controls_overlay(&self.app, overlay_settings);
    }

    fn render(&mut self) {
        let Some(window) = self.window.as_ref() else {
            self.shell.needs_redraw = false;
            return;
        };
        let Some(renderer) = self.renderer.as_mut() else {
            self.shell.needs_redraw = false;
            return;
        };
        // If the session has no loaded ROM, clear the display just before
        // rendering to guard against a stale frame that the emuthread may
        // have written into shared_fb between the last clear_display() call
        // and now (race between Unload processing and render_frame completion).
        if !self.session.loaded() {
            self.session.clear_display();
        }
        self.session.swap_frame_buffer();
        let window_size = SurfaceSize::new(window.inner_size().width, window.inner_size().height);
        if renderer.size() != window_size {
            renderer.resize(window_size);
        }
        let Some(fb) = self.session.frame_buffer() else {
            self.shell.needs_redraw = false;
            return;
        };
        match renderer.render(fb) {
            RenderResult::Presented => {
                self.shell
                    .on_frame_presented(self.session.metrics().frame_counter);
                self.request_redraw();
            }
            RenderResult::Skipped => {
                self.shell.needs_redraw = true;
            }
            RenderResult::Error => {
                log::warn!("render: renderer reported an error");
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
                    self.session.apply_input_event(event);
                    self.request_redraw();
                }
            }
        }
    }

    fn sync_touch_target(
        &mut self,
        touch_id: u64,
        next_target: Option<(AttachmentId, DigitalControlId)>,
    ) {
        let previous = self.active_touches.get(&touch_id).copied();
        if previous == next_target {
            return;
        }
        if let Some(previous) = previous {
            self.apply_touch_actions(vec![TouchOverlayAction::Input(
                DigitalInputEvent::released(previous.0, previous.1),
            )]);
            self.active_touches.remove(&touch_id);
        }
        if let Some(next) = next_target {
            self.apply_touch_actions(vec![TouchOverlayAction::Input(DigitalInputEvent::pressed(
                next.0, next.1,
            ))]);
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
                if touch.phase == TouchPhase::Started
                    && next_target.is_some()
                    && self.session.settings_snapshot().local.touch_overlay.haptics
                {
                    perform_control_haptic(&self.app);
                }
                self.sync_touch_target(touch.id, next_target);
            }
            TouchPhase::Ended | TouchPhase::Cancelled => {
                self.sync_touch_target(touch.id, None);
            }
        }
    }

    fn exec(&mut self, cmd: SessionCommand) -> Option<SessionCommandOutcome> {
        match self.session.run_command(cmd) {
            Ok(o) => {
                if o.needs_redraw {
                    self.request_redraw();
                }
                Some(o)
            }
            Err(e) => {
                log::warn!("command {cmd:?} failed: {e}");
                None
            }
        }
    }
}

fn notify_rom_loaded(app: &AndroidApp, system_id: &str) {
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr() as _) };
    let _: Result<(), jni::errors::Error> = vm.attach_current_thread(|env| {
        let activity_raw = app.activity_as_ptr() as jni::sys::jobject;
        let activity = unsafe { jni::objects::JObject::from_raw(env, activity_raw) };
        let system_id = env.new_string(system_id)?;
        env.call_method(
            &activity,
            jni_str!("notifyRomLoaded"),
            jni_sig!("(Ljava/lang/String;)V"),
            &[jni::objects::JValue::Object(system_id.as_ref())],
        )?;
        Ok(())
    });
}

fn perform_control_haptic(app: &AndroidApp) {
    let app = app.clone();
    let callback_app = app.clone();
    app.run_on_java_main_thread(Box::new(move || {
        let vm = unsafe { jni::JavaVM::from_raw(callback_app.vm_as_ptr() as _) };
        let _: Result<(), jni::errors::Error> = vm.attach_current_thread(|env| {
            let activity_raw = callback_app.activity_as_ptr() as jni::sys::jobject;
            let activity = unsafe { jni::objects::JObject::from_raw(env, activity_raw) };
            env.call_method(
                &activity,
                jni_str!("performControlHaptic"),
                jni_sig!("()V"),
                &[],
            )?;
            Ok(())
        });
    }));
}

impl FrontendSession for AndroidFrontend {
    fn run_command(&mut self, command: SessionCommand) {
        self.exec(command);
    }

    fn pause(&mut self) {
        self.exec(SessionCommand::Pause);
    }

    fn resume(&mut self) {
        self.exec(SessionCommand::Resume);
    }

    fn toggle_pause(&mut self) {
        self.exec(SessionCommand::TogglePause);
    }

    fn save_active_slot(&mut self) {
        self.exec(SessionCommand::SaveActiveSlotOrNew);
    }

    fn load_active_slot(&mut self) -> bool {
        self.exec(SessionCommand::LoadActiveSlot)
            .unwrap_or_default()
            .executed
    }

    fn select_next_slot(&mut self) {
        self.exec(SessionCommand::SelectNextSlot);
    }

    fn select_previous_slot(&mut self) {
        self.exec(SessionCommand::SelectPreviousSlot);
    }

    fn load_slot(&mut self, slot_id: u64) -> bool {
        self.exec(SessionCommand::LoadSlot(slot_id))
            .unwrap_or_default()
            .executed
    }

    fn save_slot(&mut self, slot_id: u64) {
        self.exec(SessionCommand::SaveSlot(slot_id));
    }

    fn delete_slot(&mut self, slot_id: u64) {
        self.exec(SessionCommand::DeleteSlot(slot_id));
    }

    fn select_slot(&mut self, slot_id: u64) {
        self.exec(SessionCommand::SelectActiveSlot(slot_id));
    }

    fn create_slot(&mut self) {
        self.exec(SessionCommand::CreateSlot);
    }

    fn reset(&mut self) {
        self.exec(SessionCommand::Reset);
    }

    fn apply_settings(
        &mut self,
        settings: SettingsSnapshot,
    ) -> Result<SettingsResult, SessionError> {
        let plan = self.session.apply_settings(settings)?;
        Ok(SettingsResult {
            renderer_needs_rebuild: plan.renderer_rebuild_required,
            fullscreen_default_changed: plan.fullscreen_default_changed,
            scaling_changed: plan.scaling_changed,
        })
    }

    fn set_fullscreen_default(
        &mut self,
        _fullscreen: bool,
    ) -> Result<SettingsResult, SessionError> {
        Ok(SettingsResult::default())
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
        bridge::reset_transient();
        picker::reset_transient();
        settings::reset_transient();
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
                self.session.clear_input();
                self.physical_pressed.clear();
            }
            WindowEvent::Resized(size) => {
                log::info!("window_event: resized to {}x{}", size.width, size.height);
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.resize(SurfaceSize::new(size.width, size.height));
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
        if let Some(result) = picker::take_result() {
            self.handle_picker_result(event_loop, result);
        }
        if let Some(result) = settings::take_result() {
            self.handle_settings_result(result);
        }
        for action in menu::take_actions() {
            self.handle_menu_action(action);
        }
        self.maybe_refresh_title(now);

        if let Some(window) = self.window.as_ref() {
            let frame_counter = self.session.metrics().frame_counter;
            if self.shell.wants_redraw(frame_counter) {
                window.request_redraw();
            }
            // Render loop is self-sustaining via request_redraw() after each present.
            event_loop.set_control_flow(ControlFlow::Wait);
        } else {
            if self.is_resumed || self.foreground_resume_pending {
                event_loop.set_control_flow(ControlFlow::Poll);
            } else {
                event_loop.set_control_flow(ControlFlow::Wait);
            }
        }
    }
}

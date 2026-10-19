mod bridge;
mod dialog_coordinator;
mod dialog_ownership;
mod error_dialog;
mod input_router;
mod lifecycle;
mod menu;
mod messages;
mod osd;
mod picker;
mod render_sync;
mod saf;
mod settings;
mod storage;
mod touch_layout;

use std::{ffi::c_void, path::PathBuf, rc::Rc, sync::Arc, time::Instant};

use jni::{jni_sig, jni_str};
use nerust_core_traits::{audio::AudioBackendRegistry, factory::load::MediaObject};
use nerust_gui_runtime::{
    settings::{
        BackendPresentationCapabilities, HostBackendCapabilities, HostWindowCapabilities,
        SettingsPaths, SettingsSnapshot,
    },
    shell::NativeShellState,
};
use nerust_gui_shell::{
    registry::SystemRegistry,
    session::{
        SessionError, SessionHandle,
        access::{PauseControl, SessionControl, SettingsResult, SlotControl},
        commands::{SessionCommand, SessionCommandOutcome},
        notification::FailureOrigin,
        osd::TickMetrics,
    },
};
use nerust_render_traits::{SurfaceSize, renderer::GpuFactory};
use nerust_settings_core::model::{local::RumbleTarget, shared::StoragePolicy};
use sha2::{Digest, Sha256};
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    platform::android::{EventLoopBuilderExtAndroid, activity::AndroidApp},
    window::WindowId,
};

use self::{
    dialog_coordinator::DialogCoordinator,
    dialog_ownership::DialogOwnership,
    input_router::InputRouter,
    lifecycle::Lifecycle,
    messages::{FrontendEvent, MenuAction, RomPickerResult, SettingsDialogResult},
    render_sync::RenderSync,
    settings::{AndroidSettings, SettingsApplier, SettingsPlan},
    storage::{AndroidStorage, LastMediaReference},
};
use crate::error_dialog_controller::{DismissTransition, SettingsClosedTransition, ShowTransition};

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
            jni::NativeMethod::from_raw_parts(
                jni_str!("onActivityDestroyed"),
                jni_str!("()V"),
                bridge::Java_io_github_chalharu_nerust_MainActivity_onActivityDestroyed
                    as *mut c_void,
            ),
            jni::NativeMethod::from_raw_parts(
                jni_str!("onCartridgeAcceleration"),
                jni_str!("(FF)V"),
                bridge::Java_io_github_chalharu_nerust_MainActivity_onCartridgeAcceleration
                    as *mut c_void,
            ),
            jni::NativeMethod::from_raw_parts(
                jni_str!("onSystemInsetsChanged"),
                jni_str!("([I)V"),
                bridge::Java_io_github_chalharu_nerust_MainActivity_onSystemInsetsChanged
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
    // Toast must be created/shown on the Java main thread; this function
    // is called from the winit event-loop thread where it would otherwise
    // fail silently (no Looper) and hide save/load failures from the user.
    let message = message.to_string();
    bridge::call_on_activity(app, "show Android toast", move |env, activity| {
        let toast_class = env.find_class(jni_str!("android/widget/Toast"))?;
        let text = env.new_string(&message)?;
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
        env.call_method(&toast_obj, jni_str!("show"), jni_sig!("()V"), &[])?;
        Ok(())
    });
}

fn configure_cartridge_peripherals(
    app: &AndroidApp,
    motion_enabled: bool,
    rumble_enabled: bool,
    rumble_strength_percent: u8,
    rumble_target: RumbleTarget,
) {
    let target = match rumble_target {
        RumbleTarget::Auto => "auto",
        RumbleTarget::Handset => "handset",
        RumbleTarget::Controller => "controller",
    }
    .to_string();
    bridge::call_on_activity(
        app,
        "configure Android cartridge peripherals",
        move |env, activity| {
            let target = env.new_string(&target)?;
            env.call_method(
                &activity,
                jni_str!("configureCartridgePeripherals"),
                jni_sig!("(ZZILjava/lang/String;)V"),
                &[
                    jni::objects::JValue::Bool(motion_enabled),
                    jni::objects::JValue::Bool(rumble_enabled),
                    jni::objects::JValue::Int(i32::from(rumble_strength_percent)),
                    jni::objects::JValue::Object(target.as_ref()),
                ],
            )?;
            Ok(())
        },
    );
}

fn set_cartridge_rumble(app: &AndroidApp, intensity: u8) {
    bridge::call_on_activity(
        app,
        "update Android cartridge rumble",
        move |env, activity| {
            env.call_method(
                &activity,
                jni_str!("setCartridgeRumble"),
                jni_sig!("(I)V"),
                &[jni::objects::JValue::Int(i32::from(intensity))],
            )?;
            Ok(())
        },
    );
}

struct AndroidFrontend {
    app: AndroidApp,
    session: SessionHandle,
    storage: AndroidStorage,
    shell: NativeShellState,
    gpu_factory: Rc<dyn GpuFactory>,
    input: InputRouter,
    lifecycle: Lifecycle,
    dialogs: DialogCoordinator,
    render_sync: RenderSync,
    is_resumed: bool,
    pending_storage_settings: Option<SettingsSnapshot>,
    pending_legacy_digest: Option<[u8; 32]>,
    last_peripheral_config: Option<(bool, bool, u8, RumbleTarget)>,
    last_rumble_intensity: Option<u8>,
    osd: osd::AndroidOsd,
    event_scratch: Vec<crate::android::messages::FrontendEvent>,
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
            gpu_factory,
            input: InputRouter::new(),
            lifecycle: Lifecycle::new(restore_pending),
            dialogs: DialogCoordinator::new(DialogOwnership::new()),
            render_sync: RenderSync::new(),
            is_resumed: false,
            pending_storage_settings: None,
            pending_legacy_digest: None,
            last_peripheral_config: None,
            last_rumble_intensity: None,
            osd: osd::AndroidOsd::new(),
            event_scratch: Vec::new(),
        };
        if frontend.lifecycle.restore_pending() {
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
        settings::apply_screen_orientation(&self.app, current.screen_orientation);
    }

    fn sync_cartridge_peripherals(&mut self) {
        let handles = self.session.host_peripherals();
        let requested = handles
            .accelerometer
            .as_ref()
            .is_some_and(|handle| handle.demand().requested);
        let settings = &self.session.settings_snapshot().local.cartridge_peripherals;
        let motion_active = self.is_resumed && requested && settings.motion_enabled;
        if !motion_active && let Some(accelerometer) = handles.accelerometer.as_ref() {
            accelerometer.clear();
        }
        bridge::bind_accelerometer(
            motion_active
                .then(|| handles.accelerometer.as_ref().cloned())
                .flatten(),
        );

        let config = (
            motion_active,
            settings.rumble_enabled,
            settings.rumble_strength_percent.min(100),
            settings.rumble_target,
        );
        if self.last_peripheral_config != Some(config) {
            configure_cartridge_peripherals(&self.app, config.0, config.1, config.2, config.3);
            self.last_peripheral_config = Some(config);
            self.last_rumble_intensity = None;
        }

        let intensity = if self.is_resumed && settings.rumble_enabled {
            handles
                .rumble
                .as_ref()
                .map_or(0, |handle| handle.snapshot().state.intensity)
        } else {
            0
        };
        if self.last_rumble_intensity != Some(intensity) {
            set_cartridge_rumble(&self.app, intensity);
            self.last_rumble_intensity = Some(intensity);
        }
    }

    fn stop_cartridge_peripherals(&mut self) {
        if let Some(accelerometer) = self.session.host_peripherals().accelerometer.as_ref() {
            accelerometer.clear();
        }
        bridge::bind_accelerometer(None);
        configure_cartridge_peripherals(&self.app, false, false, 0, RumbleTarget::Auto);
        set_cartridge_rumble(&self.app, 0);
        self.last_peripheral_config = None;
        self.last_rumble_intensity = None;
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
        self.input
            .reset_touch_tracking(&mut self.session, &self.app);
        self.input.forget_physical_pressed();
        self.session
            .set_active_system(system_id.as_ref())
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
        self.notify_rom_loaded(&system_id.to_string());
        log::info!("load_media: session ready for system={system_id}");
        Ok(())
    }

    /// Tell Kotlin a ROM loaded. JNI failures route to the error dialog
    /// queue (Bridge origin) instead of staying log-only.
    fn notify_rom_loaded(&mut self, system_id: &str) {
        let vm = unsafe { jni::JavaVM::from_raw(self.app.vm_as_ptr() as _) };
        let result = bridge::with_java_env(&vm, |env| {
            let activity_raw = self.app.activity_as_ptr() as jni::sys::jobject;
            let activity = unsafe { jni::objects::JObject::from_raw(env, activity_raw) };
            let system_id = env.new_string(system_id)?;
            env.call_method(
                &activity,
                jni_str!("notifyRomLoaded"),
                jni_sig!("(Ljava/lang/String;)V"),
                &[jni::objects::JValue::Object(system_id.as_ref())],
            )?;
            Ok::<(), jni::errors::Error>(())
        });
        if let Err(error) = result {
            log::warn!("failed to notify Android ROM load: {error:?}");
            self.session.report_external_error(
                FailureOrigin::Bridge,
                format!("failed to notify Android ROM load: {error:?}"),
            );
        }
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
                    self.session.report_external_error(
                        nerust_gui_shell::session::notification::FailureOrigin::Picker,
                        error,
                    );
                }
            }
            RomPickerResult::TreeSelected(uri) => {
                let Some(mut next) = self.pending_storage_settings.take() else {
                    log::warn!("ignoring unexpected Android directory picker result");
                    return;
                };
                next.shared.persistence.storage_document_tree_uri = Some(uri);
                match self.apply_settings(next) {
                    Ok(_) => {
                        self.refresh_dialog_caches();
                        self.render_sync.request_redraw(&mut self.shell);
                    }
                    Err(error) => {
                        log::error!("failed to apply Android storage directory: {error}");
                        self.session.report_external_error(
                            nerust_gui_shell::session::notification::FailureOrigin::Picker,
                            "Selected directory could not be used".to_string(),
                        );
                    }
                }
            }
            RomPickerResult::Cancelled => {
                self.pending_storage_settings = None;
                log::info!("handle_picker_result: picker dismissed");
            }
        }
        // Transient UI summons IME and system bars: the cached insets
        // key (size+density) cannot see those, so re-query next frame.
        self.osd.invalidate_system_insets();
    }

    fn handle_settings_result(&mut self, result: SettingsDialogResult, tick: TickMetrics) {
        // Settings closed (applied or dismissed): flush the error queue
        // and resume an orphaned pause. The gate reads DialogOwnership
        // (bridge truth); no cached flag exists to go stale.
        let is_applied = matches!(result, SettingsDialogResult::Applied(_));
        if self.dialogs.is_settings_open() && !is_applied {
            // Stale synthesized dismiss racing a suspend-then-rerequest:
            // the dialog is visually up, so this result belongs to a
            // dead flight. Ignore it; the live flight's result arrives
            // with its own request id.
            log::debug!("handle_settings_result: stale result while dialog open; ignoring");
            return;
        }
        if self.dialogs.is_settings_open() {
            // Applied racing a live re-request (suspend cleared the
            // old pending, the rerequest minted a new one before this
            // ran): the values are real user input, so apply them even
            // though the new dialog shows pre-apply content. Error
            // flush stays shielded below; the user re-saves if needed.
            log::warn!("handle_settings_result: applying settings while a newer dialog is open");
        } else {
            match self
                .dialogs
                .on_settings_closed(self.osd.manager_mut(), self.is_resumed, tick)
            {
                SettingsClosedTransition::Show { auto_pause } => {
                    if auto_pause {
                        self.pause();
                    }
                    if let Some(detail) = self.dialogs.current_detail() {
                        error_dialog::request_show_error_dialog(&self.app, detail);
                    }
                }
                SettingsClosedTransition::Resume => self.resume(),
                SettingsClosedTransition::Idle => {}
            }
        }
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
        // Pure plan step; the frontend only executes the outcome.
        let plan = SettingsApplier::plan(
            &values,
            &current,
            self.session.settings_snapshot(),
            self.session.registry(),
        );
        match plan {
            SettingsPlan::Invalid { reason } => {
                log::error!("{reason}");
                self.session
                    .report_external_error(FailureOrigin::Bridge, reason);
            }
            SettingsPlan::NeedDocumentTree { next } => {
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
                        self.session
                            .report_external_error(FailureOrigin::Bridge, error);
                    }
                }
            }
            SettingsPlan::Apply { next } => self.execute_settings_apply(next),
        }
    }

    /// Execute an `Apply` plan: apply, rebuild renderer/overlay, refresh
    /// caches. JNI and session failures route to the error queue.
    fn execute_settings_apply(&mut self, next: SettingsSnapshot) {
        match self.apply_settings(next) {
            Ok(result) => {
                if result.renderer_needs_rebuild {
                    // If Android has already dropped the surface, keep the renderer absent here;
                    // `ensure_window` will rebuild it on the next resume with the updated settings.
                    if let Some(window) = self.render_sync.cloned_window() {
                        self.render_sync.rebuild_renderer(
                            window,
                            &self.session,
                            &self.gpu_factory,
                            &mut self.osd,
                            &mut self.shell,
                        );
                    }
                }
                // Push touch-overlay (and FPS-label) settings to the live
                // chrome: without this the saved snapshot and the visible
                // overlay diverge, so a changed opacity/scale/visibility
                // looks "ignored" until the next ROM load or resize.
                self.input.rebuild_overlay(
                    &self.session,
                    &self.app,
                    self.render_sync.window_size(),
                );
                self.render_sync.request_redraw(&mut self.shell);
                // Settings changed – refresh cached settings for sync dialogs.
                self.refresh_dialog_caches();
            }
            Err(error) => {
                let detail = format!("failed to apply Android settings: {error}");
                log::error!("{detail}");
                self.session
                    .report_external_error(FailureOrigin::Bridge, detail);
            }
        }
    }

    fn save_lifecycle_state(&mut self) {
        // Auto-pause only with a pausable session; the predicate
        // pins the contract (host-tested in `lifecycle_decision`).
        if self.lifecycle.should_auto_pause(&self.session) {
            self.pause();
            self.lifecycle.set_auto_paused(true);
            log::info!("save_lifecycle_state: auto-paused session");
        }
        self.session.clear_input();
        self.input
            .reset_touch_tracking(&mut self.session, &self.app);
        self.input.forget_physical_pressed();
        self.lifecycle
            .save_for_suspend(&mut self.session, &self.storage);
    }

    fn release_window_resources(&mut self) {
        self.release_surface_resources();
        self.render_sync.release_window_resources();
    }

    fn release_surface_resources(&mut self) {
        self.render_sync.release_renderer();
        self.input.rebuild_overlay(&self.session, &self.app, None);
        self.input
            .reset_touch_tracking(&mut self.session, &self.app);
        self.input.release_physical_tracking(&mut self.session);
        self.shell.needs_redraw = true;
    }

    fn handle_surface_close(&mut self) {
        log::warn!("handle_surface_close: surface closed");
        self.save_lifecycle_state();
        self.release_window_resources();
        if self.is_resumed {
            self.lifecycle.begin_foreground_resume();
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
                // JNI picker failures surface in the error dialog queue,
                // not just logcat: mobile has no stderr to inspect.
                self.session
                    .report_external_error(FailureOrigin::Bridge, error);
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
                log::debug!("Android settings dialog already open");
            }
            Err(error) => {
                log::error!("{error}");
            }
        }
        // Bridge truth, not the request outcome: the sync show path
        // leaves this dialog up while the request reports "already".
        // No cached flag: every gate queries DialogOwnership instead.
    }

    fn handle_menu_action(&mut self, event: FrontendEvent, tick: TickMetrics) {
        log::info!("AndroidFrontend::handle_menu_action: {:?}", event);
        // Exhaustive over the event: a new FrontendEvent variant fails
        // to compile here instead of silently becoming a dialog dismiss.
        // `tick` is threaded through from the single per-pass metrics
        // lock; handlers must not re-lock for their own gates.
        match event {
            FrontendEvent::Menu(action) => match action {
                MenuAction::ControllerInput {
                    device_id,
                    key,
                    pressed,
                } => {
                    self.input.apply_physical_controller_input(
                        &mut self.session,
                        device_id,
                        key,
                        pressed,
                    );
                }
                MenuAction::Exit => {
                    if self.session.tick_metrics().loaded {
                        self.session.clear_hidden_lifecycle_state();
                        self.storage.clear_restore_pending();
                        if let Err(error) = self.session.unload() {
                            log::warn!("failed to unload session on exit: {error}");
                        }
                    }
                    self.session.flush_before_exit();
                    let vm = unsafe { jni::JavaVM::from_raw(self.app.vm_as_ptr() as _) };
                    if let Err(error) = bridge::with_java_env(&vm, |env| {
                        let activity_raw = self.app.activity_as_ptr() as jni::sys::jobject;
                        let activity =
                            unsafe { jni::objects::JObject::from_raw(env, activity_raw) };
                        env.call_method(&activity, jni_str!("finish"), jni_sig!("()V"), &[])?;
                        Ok::<(), jni::errors::Error>(())
                    }) {
                        log::warn!("failed to finish Android activity on exit: {error:?}");
                        self.session.report_external_error(
                            FailureOrigin::Bridge,
                            format!("failed to finish Android activity on exit: {error:?}"),
                        );
                    }
                }
                MenuAction::Unload => {
                    if self.session.tick_metrics().loaded {
                        self.session.clear_hidden_lifecycle_state();
                        self.storage.clear_restore_pending();
                        let _ = self.session.unload();
                    }
                    self.session.clear_display();
                    self.render_sync.request_redraw(&mut self.shell);
                }
                MenuAction::LoadState => {
                    let _ = self.exec(SessionCommand::LoadActiveSlot);
                }
                MenuAction::OpenRom => self.request_open_rom(),
                MenuAction::OpenSettings => self.request_settings_dialog(),
                MenuAction::OrientationChanged => {
                    // Screen rotation: rebuild the touch overlay (hit zones)
                    // for the new window size even if winit did not deliver
                    // a Resized event. Idempotent with the Resized path.
                    self.input.rebuild_overlay(
                        &self.session,
                        &self.app,
                        self.render_sync.window_size(),
                    );
                    self.render_sync.request_redraw(&mut self.shell);
                }
                MenuAction::Reset => self.reset(),
                MenuAction::SaveState => {
                    let _ = self.exec(SessionCommand::SaveActiveSlotOrNew);
                }
                MenuAction::TogglePause => self.toggle_pause(),
            },
            FrontendEvent::ErrorDialogDismissed => self.handle_error_dialog_dismissed(tick),
        }
    }

    /// Show the oldest queued error unless suspended, already showing,
    /// or settings owns the screen. Bridge truth via DialogOwnership:
    /// the sync JNI path shows the settings dialog before the event
    /// loop drains the open event, and Kotlin also refuses the eviction;
    /// both layers guard the same race.
    /// Takes the pass tick so the caller pays one metrics lock, not
    /// one per dialog gate.
    fn maybe_show_error_dialog(&mut self, tick: TickMetrics) {
        let request = self
            .dialogs
            .poll_error_dialog(self.osd.manager_mut(), self.is_resumed, tick);
        let Some(transition) = request else {
            return;
        };
        // Shell opened before the fire-and-forget request; a failed
        // bridge synthesizes dismiss, so the cap is a backstop.
        let ShowTransition::Present { auto_pause } = transition;
        if auto_pause {
            self.pause();
        }
        if let Some(detail) = self.dialogs.current_detail() {
            error_dialog::request_show_error_dialog(&self.app, detail);
        }
    }

    /// Dismiss hook from the Kotlin dialog. Resumes only our own
    /// auto-pause, then shows the next queued error, if any.
    /// `tick` is the pass tick: this handler must not take a second
    /// metrics lock (the old re-lock broke the single-lock-per-pass
    /// contract in `about_to_wait`).
    fn handle_error_dialog_dismissed(&mut self, tick: TickMetrics) {
        // Bridge truth here: a settings dialog may have opened over the
        // error since the last event.
        let transition = self
            .dialogs
            .on_dismissed(self.osd.manager_mut(), self.is_resumed);
        match transition {
            DismissTransition::Idle => {}
            DismissTransition::Reshow => {
                self.maybe_show_error_dialog(tick);
            }
            DismissTransition::ResumeAndReshow => {
                self.resume();
                self.maybe_show_error_dialog(tick);
            }
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
        // Stale batches straddle no ROM boundary: requeue the unacked
        // head so the previous ROM's error never describes the new one.
        // The pause flag survives, so the gate below holds the resume
        // until the requeued dialog is dismissed.
        self.dialogs.on_suspended(self.osd.manager_mut());
        // Resume fresh loads and own auto-pauses, never across an error
        // auto-pause (its dismiss owns that); a manual pause predates
        // the suspend and must survive it.
        if self
            .lifecycle
            .resume_want_after_restore(restore_hidden_state, self.dialogs.auto_paused())
        {
            self.lifecycle.set_auto_paused(false);
            self.resume();
        }
        self.lifecycle.clear_restore_pending();
        self.storage.clear_restore_pending();

        // Ensure a window and renderer exist immediately when resuming so that
        // request_redraw() takes effect without requiring a user tap.
        if !self.render_sync.has_window() && self.is_resumed {
            match self.render_sync.ensure_window(
                event_loop,
                &self.session,
                &self.gpu_factory,
                &mut self.osd,
                &mut self.shell,
            ) {
                Ok(()) => {
                    log::info!("finish_rom_load: ensured window/renderer after ROM load");
                    self.input.rebuild_overlay(
                        &self.session,
                        &self.app,
                        self.render_sync.window_size(),
                    );
                }
                Err(error) => {
                    log::warn!(
                        "finish_rom_load: ensure_window failed: {error}; scheduling foreground resume"
                    );
                    self.lifecycle.begin_foreground_resume();
                }
            }
        } else if let Some(window) = self.render_sync.cloned_window() {
            // Rebuild renderer to ensure the underlying surface is fresh; dialogs or
            // system UI may have invalidated the previous surface.
            log::info!("finish_rom_load: rebuilding renderer to ensure visible surface");
            self.render_sync.rebuild_renderer(
                window,
                &self.session,
                &self.gpu_factory,
                &mut self.osd,
                &mut self.shell,
            );
            self.input
                .rebuild_overlay(&self.session, &self.app, self.render_sync.window_size());
        }

        self.refresh_dialog_caches();
        self.render_sync.request_redraw(&mut self.shell);
    }

    fn try_resume_foreground(&mut self, event_loop: &ActiveEventLoop) {
        if !self.is_resumed || !self.lifecycle.resume_pending() {
            return;
        }
        if let Some(retry_at) = self.lifecycle.retry_deadline() {
            if Instant::now() < retry_at {
                event_loop.set_control_flow(ControlFlow::WaitUntil(retry_at));
                return;
            }
            self.lifecycle.clear_retry_deadline();
        }
        log::info!(
            "try_resume_foreground: attempt {}",
            self.lifecycle.attempt_index()
        );
        match self.render_sync.ensure_window(
            event_loop,
            &self.session,
            &self.gpu_factory,
            &mut self.osd,
            &mut self.shell,
        ) {
            Ok(()) => {
                let attempt = self.lifecycle.attempt_index();
                self.lifecycle.record_success();
                if self.lifecycle.restore_pending() {
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
                                    self.session.report_external_error(
                                        nerust_gui_shell::session::notification::FailureOrigin::Restore,
                                        "Previous ROM is unavailable; open it again".to_string(),
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
                                    self.lifecycle.clear_restore_pending();
                                } else {
                                    match self
                                        .load_from_library_with_autosave(event_loop, &id, true)
                                    {
                                        Ok(()) => {
                                            log::info!(
                                                "try_resume_foreground: loaded last ROM id={id} for lifecycle restore"
                                            );
                                            // Toasts mark success only; failures surface via the error dialog.
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
                                            self.lifecycle.clear_restore_pending();
                                        }
                                    }
                                }
                            }
                            Ok(None) => {
                                log::info!("try_resume_foreground: no last ROM recorded");
                                self.session.clear_hidden_lifecycle_state();
                                self.storage.clear_restore_pending();
                                self.lifecycle.clear_restore_pending();
                            }
                            Err(error) => {
                                log::warn!(
                                    "try_resume_foreground: failed to read last ROM id: {error}"
                                );
                                self.session.clear_hidden_lifecycle_state();
                                self.storage.clear_restore_pending();
                                self.lifecycle.clear_restore_pending();
                            }
                        },
                        Err(error) => {
                            log::warn!(
                                "try_resume_foreground: failed to read last media reference: {error}"
                            );
                            self.session.report_external_error(
                                nerust_gui_shell::session::notification::FailureOrigin::Restore,
                                "Previous ROM reference is invalid; open it again".to_string(),
                            );
                            self.session.clear_hidden_lifecycle_state();
                            self.storage.clear_restore_pending();
                            self.lifecycle.clear_restore_pending();
                        }
                    }
                }
                // Resume only our own lifecycle auto-pause, never across
                // an error auto-pause (its dismiss path owns that resume).
                if self
                    .lifecycle
                    .resume_want_foreground_retry(self.dialogs.auto_paused())
                {
                    self.resume();
                    self.lifecycle.set_auto_paused(false);
                    log::info!("try_resume_foreground: resumed session after lifecycle pause");
                }
                // The OS may have killed the audio stream while
                // backgrounded regardless of pause state; re-acquire it
                // (idempotent, silent when already playing).
                self.session.restart_audio();
                log::info!("try_resume_foreground: attempt {attempt} succeeded");
                self.render_sync.request_redraw(&mut self.shell);
            }
            Err(error) => {
                let attempt = self.lifecycle.attempt_index();
                log::warn!("try_resume_foreground: attempt {attempt} failed: {error}");
                if self.lifecycle.schedule_retry(self.is_resumed)
                    && let Some(retry_at) = self.lifecycle.retry_deadline()
                {
                    event_loop.set_control_flow(ControlFlow::WaitUntil(retry_at));
                }
            }
        }
    }

    fn notification_settings(&self) -> nerust_settings_core::model::local::NotificationSettings {
        self.session.settings_snapshot().local.notifications.clone()
    }

    fn notification_language(&self) -> nerust_settings_core::model::language::AppLanguage {
        self.session.settings_snapshot().shared.general.language
    }

    fn maybe_refresh_title(&mut self, now: Instant) {
        if self.shell.should_refresh_title(now)
            && let Some(window) = self.render_sync.cloned_window()
        {
            window.set_title(&self.session.window_title(&self.session.tick_metrics()));
        }
    }

    fn exec(&mut self, cmd: SessionCommand) -> Option<SessionCommandOutcome> {
        match self.session.run_command(cmd) {
            Ok(o) => {
                if o.needs_redraw {
                    self.render_sync.request_redraw(&mut self.shell);
                }
                Some(o)
            }
            Err(error) => {
                // Mobile has no stderr: a silent None looks like
                // success. The shell already pushed a modal batch;
                // the warn keeps the transport cause in logcat.
                log::warn!("Android exec {cmd:?} failed: {error}");
                None
            }
        }
    }
}

impl PauseControl for AndroidFrontend {
    fn pause(&mut self) {
        self.exec(SessionCommand::Pause);
    }

    fn resume(&mut self) {
        self.exec(SessionCommand::Resume);
    }

    fn toggle_pause(&mut self) {
        self.exec(SessionCommand::TogglePause);
    }
}

impl SlotControl for AndroidFrontend {
    fn save_active_slot(&mut self) -> bool {
        self.exec(SessionCommand::SaveActiveSlotOrNew)
            .unwrap_or_default()
            .executed
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
}

impl SessionControl for AndroidFrontend {
    fn unload(&mut self) {
        if let Err(error) = self.session.unload() {
            log::warn!("ROM unload failed: {error}");
        }
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
        self.lifecycle.begin_foreground_resume();
        self.try_resume_foreground(event_loop);
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        log::info!("ApplicationHandler::suspended");
        self.is_resumed = false;
        self.lifecycle.cancel_foreground_resume();
        self.stop_cartridge_peripherals();
        self.save_lifecycle_state();
        bridge::reset_transient();
        picker::reset_transient();
        settings::reset_transient();
        // Transient UI dies with the activity: the coordinator releases
        // the gate and requeues the unacked head; the settings bridge
        // pending id dies with the dialog it tracked.
        self.dialogs.on_suspended(self.osd.manager_mut());
        self.release_window_resources();
    }

    fn window_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if !self.render_sync.window_id_matches(window_id) {
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
                self.input.forget_physical_pressed();
            }
            WindowEvent::Resized(size) => {
                log::info!("window_event: resized to {}x{}", size.width, size.height);
                self.render_sync
                    .resize_renderer(SurfaceSize::new(size.width, size.height));
                self.input.rebuild_overlay(
                    &self.session,
                    &self.app,
                    self.render_sync.window_size(),
                );
                self.render_sync.request_redraw(&mut self.shell);
            }
            WindowEvent::Touch(touch) => {
                self.input.handle_touch(&mut self.session, &self.app, touch);
            }
            WindowEvent::RedrawRequested => {
                let tick = self.session.tick_metrics();
                self.render_sync.render(
                    &self.app,
                    &mut self.session,
                    &mut self.osd,
                    &mut self.shell,
                    tick,
                    Instant::now(),
                );
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if bridge::take_exit_request() {
            self.session.flush_before_exit();
            self.release_window_resources();
            event_loop.exit();
            return;
        }
        let now = Instant::now();
        // One metrics lock per pass: dialog gates below thread this
        // tick instead of re-locking per predicate.
        let tick = self.session.tick_metrics();
        self.try_resume_foreground(event_loop);
        if let Some(result) = picker::take_result() {
            self.handle_picker_result(event_loop, result);
        }
        if let Some(result) = settings::take_result() {
            self.handle_settings_result(result, tick);
            // Same transient-UI reason as the picker path above.
            self.osd.invalidate_system_insets();
        }
        menu::take_events_into(&mut self.event_scratch);
        // Two-lane queue, no sort: priority acks (dialog dismissals)
        // drain first in arrival order, then the menu lane.
        let mut pending = std::mem::take(&mut self.event_scratch);
        for event in pending.drain(..) {
            self.handle_menu_action(event, tick);
        }
        self.event_scratch = pending;
        self.sync_cartridge_peripherals();
        self.maybe_refresh_title(now);
        let settings = self.notification_settings();
        let language = self.notification_language();
        if self
            .osd
            .poll(&mut self.session, &settings, language, now, tick)
        {
            self.render_sync.request_redraw(&mut self.shell);
        }
        self.maybe_show_error_dialog(tick);

        if self.render_sync.has_window() {
            if self.shell.wants_redraw(tick.frame_counter)
                && let Some(window) = self.render_sync.cloned_window()
            {
                window.request_redraw();
            }
            // Render loop is self-sustaining via request_redraw() after each present.
            // OSD expiry still needs a wake while paused, when no frames arrive.
            if let Some(deadline) = self.osd.wakeup(now) {
                event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
            } else {
                event_loop.set_control_flow(ControlFlow::Wait);
            }
        } else {
            if self.is_resumed || self.lifecycle.resume_pending() {
                event_loop.set_control_flow(ControlFlow::Poll);
            } else {
                event_loop.set_control_flow(ControlFlow::Wait);
            }
        }
    }
}

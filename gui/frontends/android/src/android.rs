mod bridge;
mod chrome_port;
mod dialog_coordinator;
mod error_dialog;
mod foreground_controller;
mod input_router;
mod key_tracker;
mod lifecycle;
mod media_loader;
mod menu;
mod osd;
mod peripheral_sync;
mod picker;
mod render_sync;
mod saf;
mod settings;
mod settings_bridge;
mod settings_codec;
mod settings_model;
mod settings_schema;
mod storage;
mod touch_layout;
mod touch_tracker;

use std::{ffi::c_void, path::PathBuf, rc::Rc, sync::Arc, time::Instant};

use jni::{jni_sig, jni_str};
use nerust_core_traits::audio::AudioBackendRegistry;
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
        access::{PauseControl, ResetControl, SessionControl, SettingsResult, SlotIo, SlotNav},
        commands::{SessionCommand, SessionCommandOutcome},
        notification::FailureOrigin,
        osd::TickMetrics,
        persistence::PersistenceBackends,
    },
};
use nerust_render_traits::{SurfaceSize, renderer::PipelineFactory};
use nerust_settings_core::model::shared::StoragePolicy;
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    platform::android::{EventLoopBuilderExtAndroid, activity::AndroidApp},
    window::WindowId,
};

use self::{
    bridge::RomPickerResult,
    dialog_coordinator::{DialogCoordinator, ScreenSource},
    input_router::InputRouter,
    lifecycle::Lifecycle,
    peripheral_sync::PeripheralSync,
    render_sync::RenderSync,
    settings::{AndroidSettings, SettingsApplier, SettingsDialogResult, SettingsPlan},
    storage::AndroidStorage,
};
use crate::{
    bridge_events::{FrontendEvent, MenuAction},
    error_dialog_controller::{DismissTransition, SettingsClosedTransition, ShowTransition},
};

pub(super) fn register_main_activity_natives(
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
                jni_str!("onErrorDialogDismissed"),
                jni_str!("(J)V"),
                menu::Java_io_github_chalharu_nerust_MainActivity_onErrorDialogDismissed
                    as *mut c_void,
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
                settings_bridge::Java_io_github_chalharu_nerust_MainActivity_onSettingsDialogResult
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

pub(super) fn run(
    app: AndroidApp,
    system_registry: Arc<SystemRegistry>,
    audio_registry: Arc<AudioBackendRegistry>,
    gpu_factory: Rc<dyn PipelineFactory>,
) -> Result<(), String> {
    // Best-effort re-registration from the native thread; the system classloader
    // may fail it. Primary registration happens in JNI_OnLoad.
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr() as _) };
    if let Err(error) = vm.attach_current_thread(register_main_activity_natives) {
        log::warn!("native re-registration skipped (expected on native thread): {error:?}");
    }

    let frontend_app = app.clone();
    let init_fail_app = app.clone();
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
    bridge::clear_queues();
    event_loop.set_control_flow(ControlFlow::Wait);

    let mut state = match AndroidFrontend::try_new(
        frontend_app,
        storage,
        storage_root.join("nerust"),
        system_registry,
        audio_registry,
        gpu_factory,
    ) {
        Ok(state) => state,
        Err(error) => {
            // No event loop exists yet, so no error-dialog queue to drain.
            // Toast the cause, then finish the activity instead of aborting.
            log::error!("android::run: frontend init failed: {error}");
            let message = format!("Startup failed: {error}");
            show_toast(&init_fail_app, &message);
            bridge::post_to_activity(
                &init_fail_app,
                "finish Android activity after init failure",
                |env, activity| {
                    bridge::call_bridge_method(
                        env,
                        &activity,
                        &bridge::methods::FINISH_ACTIVITY,
                        &[],
                    )?;
                    Ok(())
                },
                None,
            );
            return Err(error);
        }
    };
    log::info!("android::run: entering Android event loop");
    event_loop
        .run_app(&mut state)
        .map_err(|error| format!("Android event loop failed: {error}"))
}

fn show_toast(app: &AndroidApp, message: &str) {
    // Toasts need the Java main thread; the event-loop thread has no Looper.
    // A silent failure would hide save/load failures from the user.
    let message = message.to_string();
    bridge::post_to_activity(
        app,
        "show Android toast",
        move |env, activity| {
            let toast_class = env.find_class(jni_str!("android/widget/Toast"))?;
            let text = env.new_string(&message)?;
            let toast = env.call_static_method(
                &toast_class,
                jni_str!("makeText"),
                jni_sig!(
                    "(Landroid/content/Context;Ljava/lang/CharSequence;I)Landroid/widget/Toast;"
                ),
                &[
                    jni::objects::JValue::Object(&activity),
                    jni::objects::JValue::Object(text.as_ref()),
                    jni::objects::JValue::Int(0),
                ],
            )?;
            let toast_obj = toast.l()?;
            env.call_method(&toast_obj, jni_str!("show"), jni_sig!("()V"), &[])?;
            Ok(())
        },
        None,
    );
}

/// Injected screen truth: the settings bridge pending id.
/// The coordinator queries this instead of holding a cached bool.
struct FrontendScreen;

impl ScreenSource for FrontendScreen {
    fn is_settings_open(&self) -> bool {
        settings::is_dialog_open()
    }
}

/// Session-owned core: handle, storage, shell, and GPU factory.
struct SessionCore {
    session: SessionHandle,
    storage: AndroidStorage,
    shell: NativeShellState,
    gpu_factory: Rc<dyn PipelineFactory>,
}

/// Live chrome: input routing, peripherals, and the OSD layer.
struct Chrome {
    input: InputRouter,
    peripherals: PeripheralSync,
    osd: osd::AndroidOsd,
}

/// Foreground flow: lifecycle, dialogs, and window/renderer sync.
struct Flow {
    lifecycle: Lifecycle,
    dialogs: DialogCoordinator<FrontendScreen>,
    render_sync: RenderSync,
}

struct AndroidFrontend {
    app: AndroidApp,
    core: SessionCore,
    chrome: Chrome,
    flow: Flow,
    is_resumed: bool,
    pending_storage_settings: Option<SettingsSnapshot>,
    pending_legacy_digest: Option<[u8; 32]>,
    event_scratch: Vec<FrontendEvent>,
}

impl AndroidFrontend {
    /// Fallible constructor: returns the error so `run()` can toast + finish the activity.
    /// Corrupted settings files are quarantined and replaced with defaults.
    fn try_new(
        app: AndroidApp,
        storage: AndroidStorage,
        settings_root: PathBuf,
        system_registry: Arc<SystemRegistry>,
        audio_registry: Arc<AudioBackendRegistry>,
        gpu_factory: Rc<dyn PipelineFactory>,
    ) -> Result<Self, String> {
        log::info!("AndroidFrontend::try_new: building frontend state");
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
        let mut session = Self::create_session_with_recovery(
            capabilities,
            &system_registry,
            &audio_registry,
            settings_paths,
        )?;
        session.configure_persistence_backends(PersistenceBackends {
            slot: Box::new(saf::AndroidStorageBackend::new(app.clone())),
            autosave: Box::new(saf::AndroidStorageBackend::new(app.clone())),
            mapper: Box::new(saf::AndroidStorageBackend::new(app.clone())),
        });
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
            core: SessionCore {
                session,
                storage,
                shell: NativeShellState::new(),
                gpu_factory,
            },
            chrome: Chrome {
                input: InputRouter::new(),
                peripherals: PeripheralSync::new(),
                osd: osd::AndroidOsd::new(),
            },
            flow: Flow {
                lifecycle: Lifecycle::new(restore_pending),
                dialogs: DialogCoordinator::new(FrontendScreen),
                render_sync: RenderSync::new(),
            },
            is_resumed: false,
            pending_storage_settings: None,
            pending_legacy_digest: None,
            event_scratch: Vec::new(),
        };
        if frontend.flow.lifecycle.restore_pending() {
            log::info!(
                "AndroidFrontend::try_new: restore_pending flag found; will attempt lifecycle state restore at foreground resume"
            );
        }
        frontend.refresh_dialog_caches();
        log::info!("AndroidFrontend::try_new: ready");
        Ok(frontend)
    }

    /// Session creation with corrupted-settings recovery: quarantine and retry once with defaults.
    /// One bad write cannot wedge every future launch.
    fn create_session_with_recovery(
        capabilities: HostBackendCapabilities,
        system_registry: &Arc<SystemRegistry>,
        audio_registry: &Arc<AudioBackendRegistry>,
        settings_paths: SettingsPaths,
    ) -> Result<SessionHandle, String> {
        match SessionHandle::new_with_settings_paths(
            capabilities,
            system_registry.clone(),
            audio_registry.clone(),
            settings_paths.clone(),
        ) {
            Ok(session) => Ok(session),
            Err(error) => {
                log::error!(
                    "session creation failed, quarantining possibly-corrupted settings: {error}"
                );
                let stamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0);
                let corrupted = settings_paths
                    .settings_file
                    .with_extension(format!("yaml.corrupted-{stamp}"));
                match std::fs::rename(&settings_paths.settings_file, &corrupted) {
                    Ok(()) => log::warn!("renamed corrupted settings to {}", corrupted.display()),
                    Err(rename_error) if rename_error.kind() == std::io::ErrorKind::NotFound => {
                        log::debug!("no settings file to quarantine");
                    }
                    Err(rename_error) => {
                        log::warn!("failed to quarantine corrupted settings: {rename_error}");
                    }
                }
                SessionHandle::new_with_settings_paths(
                    capabilities,
                    system_registry.clone(),
                    audio_registry.clone(),
                    settings_paths,
                )
                .map_err(|retry_error| {
                    format!(
                        "failed to create Android session (initial: {error}; retry with defaults: {retry_error})"
                    )
                })
            }
        }
    }

    /// Update settings for the asynchronous, event-loop-owned dialog request.
    fn refresh_dialog_caches(&self) {
        let mut current = AndroidSettings::from_snapshot(
            self.core.session.settings_snapshot(),
            self.core.session.factories(),
        );
        current.prioritize_system(self.core.session.active_system_id());
        settings::update_cached_settings(&current);
        settings::apply_screen_orientation(&self.app, current.screen_orientation);
    }

    fn handle_picker_result(&mut self, event_loop: &ActiveEventLoop, result: RomPickerResult) {
        match result {
            RomPickerResult::Selected(uri) => {
                log::info!("handle_picker_result: picker returned URI; fetching off-thread");
                // Document/SAF I/O blocks: fetch on a worker thread and
                // dispatch the completed buffer when it publishes RomBytes.
                picker::spawn_document_fetch(self.app.clone(), uri, false);
            }
            RomPickerResult::RomBytes {
                uri,
                file_name,
                bytes,
                digest,
                restore_hidden_state,
            } => {
                log::info!(
                    "handle_picker_result: worker delivered {} bytes for {uri}",
                    bytes.len()
                );
                if let Err(error) = self.open_rom_from_bytes(
                    event_loop,
                    &uri,
                    &file_name,
                    bytes,
                    digest,
                    restore_hidden_state,
                ) {
                    log::error!("{error}");
                    self.core.session.report_external_error(
                        nerust_gui_shell::session::notification::FailureOrigin::Selection,
                        error,
                    );
                }
            }
            RomPickerResult::LibraryBytes { id, bytes, digest } => {
                log::info!(
                    "handle_picker_result: worker delivered {} bytes for library id={id}",
                    bytes.len()
                );
                if let Err(error) = self.open_library_from_bytes(event_loop, &id, bytes, digest) {
                    log::error!("{error}");
                    self.discard_restore_state();
                }
            }
            RomPickerResult::RestoreDocumentFailed(error) => {
                log::warn!("handle_picker_result: lifecycle document fetch failed: {error}");
                self.core.session.report_external_error(
                    nerust_gui_shell::session::notification::FailureOrigin::Restore,
                    "Previous ROM is unavailable; open it again".to_string(),
                );
                log::info!(
                    "handle_picker_result: preserving hidden state until the ROM URI is reconnected"
                );
            }
            RomPickerResult::RestoreLibraryFailed(error) => {
                log::warn!("handle_picker_result: lifecycle library fetch failed: {error}");
                self.discard_restore_state();
            }
            RomPickerResult::Failed(error) => {
                log::error!("{error}");
                self.core.session.report_external_error(
                    nerust_gui_shell::session::notification::FailureOrigin::Selection,
                    error,
                );
            }
            RomPickerResult::TreeSelected(uri) => {
                let Some(mut next) = self.pending_storage_settings.take() else {
                    log::warn!("ignoring unexpected Android directory picker result");
                    return;
                };
                next.shared.persistence.storage_document_tree_uri = Some(uri);
                self.execute_settings_apply(next);
            }
            RomPickerResult::Cancelled => {
                self.pending_storage_settings = None;
                log::info!("handle_picker_result: picker dismissed");
            }
        }
    }

    fn handle_settings_result(&mut self, result: SettingsDialogResult, tick: TickMetrics) {
        // Settings closed: flush the error queue and resume an orphaned pause.
        // The gate reads the injected screen source; no cached flag exists to go stale.
        let is_applied = matches!(result, SettingsDialogResult::Applied(_));
        if self.flow.dialogs.is_settings_open() && !is_applied {
            // Stale synthesized dismiss racing a suspend-then-rerequest: ignore it.
            // The live flight's result arrives with its own request id.
            log::debug!("handle_settings_result: stale result while dialog open; ignoring");
            return;
        }
        if self.flow.dialogs.is_settings_open() {
            // Applied racing a live re-request: the values are real user input, so apply them.
            // Error flush stays shielded below; the user re-saves if needed.
            log::warn!("handle_settings_result: applying settings while a newer dialog is open");
        } else {
            match self.chrome.osd.with_modal(|queue| {
                self.flow
                    .dialogs
                    .on_settings_closed(queue, self.is_resumed, tick)
            }) {
                SettingsClosedTransition::Show { auto_pause } => {
                    if auto_pause {
                        self.pause();
                    }
                    if let (Some(id), Some(detail)) = (
                        self.flow.dialogs.current_id(),
                        self.flow.dialogs.current_detail(),
                    ) {
                        error_dialog::request_show_error_dialog(&self.app, id, detail);
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
        // Thread the dialog cache instead of rebuilding from the
        // snapshot; refresh paths keep it current.
        let mut current = settings::cloned_cached_settings().unwrap_or_else(|| {
            AndroidSettings::from_snapshot(
                self.core.session.settings_snapshot(),
                self.core.session.factories(),
            )
        });
        current.prioritize_system(self.core.session.active_system_id());
        // Pure plan step; the frontend only executes the outcome.
        // Defs come from the bridge cache; the plan never re-locks.
        let defs = settings::cached_setting_defs(&current);
        let plan = SettingsApplier::plan(
            &values,
            &current,
            &defs,
            self.core.session.settings_snapshot().clone(),
            self.core.session.factories(),
        );
        match plan {
            SettingsPlan::Invalid { reason } => {
                log::error!("{reason}");
                self.core
                    .session
                    .report_external_error(FailureOrigin::Host, reason);
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
                        self.core
                            .session
                            .report_external_error(FailureOrigin::Host, error);
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
                    if let Some(window) = self.flow.render_sync.cloned_window()
                        && let Err(error) = self.flow.render_sync.rebuild_renderer(
                            window,
                            &self.core.session,
                            &self.core.gpu_factory,
                            &mut self.chrome.osd,
                            &mut self.core.shell,
                        )
                    {
                        log::warn!("settings renderer rebuild failed: {error}");
                        self.flow.lifecycle.begin_foreground_resume();
                        self.flow.lifecycle.schedule_retry(self.is_resumed);
                    }
                }
                // Push overlay and FPS-label settings to the live chrome.
                // Otherwise saved and visible settings diverge until the next load or resize.
                self.chrome.input.rebuild_overlay(
                    &self.core.session,
                    &self.app,
                    self.flow.render_sync.window_size(),
                );
                self.flow.render_sync.request_redraw(&mut self.core.shell);
                // Settings changed – refresh cached settings for sync dialogs.
                self.refresh_dialog_caches();
                // Peripheral settings may have changed: force a re-push
                // on the next pass (dirty generation).
                self.chrome.peripherals.mark_dirty();
            }
            Err(error) => {
                let detail = format!("failed to apply Android settings: {error}");
                log::error!("{detail}");
                self.core
                    .session
                    .report_external_error(FailureOrigin::Host, detail);
            }
        }
    }

    fn save_lifecycle_state(&mut self) {
        // Auto-pause only with a pausable session; the predicate
        // pins the contract (host-tested in `lifecycle_decision`).
        let tick = self.core.session.tick_metrics();
        if self.flow.lifecycle.should_auto_pause(tick.can_pause()) {
            self.pause();
            self.flow.lifecycle.set_auto_paused(true);
            log::info!("save_lifecycle_state: auto-paused session");
        }
        log::info!(
            "save_lifecycle_state: paused={} lifecycle_auto_paused={} restore_pending={}",
            tick.paused,
            self.flow.lifecycle.auto_paused(),
            self.flow.lifecycle.restore_pending()
        );
        self.core.session.clear_input();
        self.chrome
            .input
            .reset_touch_tracking(&mut self.core.session, &self.app);
        self.chrome.input.forget_physical_pressed();
        // Pure decision here; the frontend executes the marker and session I/O.
        let saved_hidden = self.core.session.save_hidden_lifecycle_state();
        match self.flow.lifecycle.save_decision(saved_hidden) {
            crate::lifecycle_decision::SuspendFileAction::Touch => {
                self.core.storage.touch_restore_pending();
                log::info!("save_lifecycle_state: hidden lifecycle state saved");
            }
            crate::lifecycle_decision::SuspendFileAction::Clear => {
                self.core.session.clear_hidden_lifecycle_state();
                self.core.storage.clear_restore_pending();
                log::info!("save_lifecycle_state: no hidden lifecycle state was produced");
            }
        }
        self.core.session.flush_before_exit();
        log::info!("save_lifecycle_state: flushed session state");
    }

    fn release_window_resources(&mut self) {
        self.release_surface_resources();
        self.flow.render_sync.release_window_resources();
    }

    fn release_surface_resources(&mut self) {
        self.flow.render_sync.release_renderer();
        self.chrome
            .input
            .rebuild_overlay(&self.core.session, &self.app, None);
        self.chrome
            .input
            .reset_touch_tracking(&mut self.core.session, &self.app);
        self.chrome
            .input
            .release_physical_tracking(&mut self.core.session);
        self.core.shell.needs_redraw = true;
    }

    fn handle_surface_close(&mut self) {
        log::warn!("handle_surface_close: surface closed");
        picker::reset_transient();
        self.save_lifecycle_state();
        self.release_window_resources();
        if self.is_resumed {
            self.flow.lifecycle.begin_foreground_resume();
        }
    }

    /// Shared unload path for Exit and Unload: drop the hidden snapshot,
    /// the restore marker, and the loaded ROM.
    fn unload_for_exit(&mut self) {
        picker::reset_transient();
        self.discard_restore_state();
        if self.core.session.tick_metrics().loaded {
            self.core.session.clear_hidden_lifecycle_state();
            self.core.storage.clear_restore_pending();
            if let Err(error) = self.core.session.unload() {
                log::warn!("failed to unload session: {error}");
            }
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
                self.core
                    .session
                    .report_external_error(FailureOrigin::Host, error);
            }
        }
    }

    fn request_settings_dialog(&mut self) {
        let mut current = settings::cloned_cached_settings().unwrap_or_else(|| {
            AndroidSettings::from_snapshot(
                self.core.session.settings_snapshot(),
                self.core.session.factories(),
            )
        });
        current.prioritize_system(self.core.session.active_system_id());
        match settings::request_show_settings_dialog(&self.app, &current) {
            Ok(true) => {}
            Ok(false) => {
                log::debug!("Android settings dialog already open");
            }
            Err(error) => {
                log::error!("{error}");
            }
        }
        // Every screen gate queries the bridge pending id.
    }

    fn handle_menu_action(&mut self, event: FrontendEvent, tick: TickMetrics) {
        log::info!("AndroidFrontend::handle_menu_action: {:?}", event);
        // Exhaustive over the event: a new variant fails to compile here.
        // Handlers must not re-lock; `tick` comes from the single per-pass lock.
        match event {
            FrontendEvent::Menu(action) => match action {
                MenuAction::ControllerInput {
                    device_id,
                    key,
                    pressed,
                } => {
                    self.chrome.input.apply_physical_controller_input(
                        &mut self.core.session,
                        device_id,
                        key,
                        pressed,
                    );
                }
                MenuAction::Exit => {
                    self.unload_for_exit();
                    self.core.session.flush_before_exit();
                    // Fire-and-forget: the activity teardown never
                    // blocks the event thread on a JNI round-trip.
                    bridge::post_to_activity(
                        &self.app,
                        "finish Android activity on exit",
                        |env, activity| {
                            bridge::call_bridge_method(
                                env,
                                &activity,
                                &bridge::methods::FINISH_ACTIVITY,
                                &[],
                            )?;
                            Ok(())
                        },
                        None,
                    );
                }
                MenuAction::Unload => {
                    self.unload_for_exit();
                    self.core.session.clear_display();
                    self.flow.render_sync.request_redraw(&mut self.core.shell);
                }
                MenuAction::LoadState => {
                    let _ = self.exec(SessionCommand::LoadActiveSlot);
                }
                MenuAction::OpenRom => self.request_open_rom(),
                MenuAction::OpenSettings => self.request_settings_dialog(),
                MenuAction::OrientationChanged => {
                    // Rotation: rebuild the touch overlay even without a Resized event.
                    // Idempotent with the Resized path.
                    self.chrome.input.rebuild_overlay(
                        &self.core.session,
                        &self.app,
                        self.flow.render_sync.window_size(),
                    );
                    self.flow.render_sync.request_redraw(&mut self.core.shell);
                }
                MenuAction::Reset => self.reset(),
                MenuAction::SaveState => {
                    let _ = self.exec(SessionCommand::SaveActiveSlotOrNew);
                }
                MenuAction::TogglePause => self.toggle_pause(),
            },
            FrontendEvent::ErrorDialogDismissed(id) => self.handle_error_dialog_dismissed(id, tick),
        }
    }

    /// Show the oldest queued error unless suspended, showing, or settings owns the screen.
    /// Takes the pass tick (one metrics lock per pass).
    fn maybe_show_error_dialog(&mut self, tick: TickMetrics) {
        let request = self.chrome.osd.with_modal(|queue| {
            self.flow
                .dialogs
                .poll_error_dialog(queue, self.is_resumed, tick)
        });
        let Some(transition) = request else {
            return;
        };
        // Shell opened before the fire-and-forget request; a failed
        // bridge synthesizes dismiss, so the cap is a backstop.
        let ShowTransition::Present { auto_pause } = transition;
        if auto_pause {
            self.pause();
        }
        if let (Some(id), Some(detail)) = (
            self.flow.dialogs.current_id(),
            self.flow.dialogs.current_detail(),
        ) {
            error_dialog::request_show_error_dialog(&self.app, id, detail);
        }
    }

    /// Dismiss hook: resumes only our own auto-pause, then shows the next queued error.
    /// `tick` is the pass tick; this handler must not take a second metrics lock.
    fn handle_error_dialog_dismissed(
        &mut self,
        id: crate::error_dialog_controller::DialogRequestId,
        tick: TickMetrics,
    ) {
        // Bridge truth here: a settings dialog may have opened over the
        // error since the last event.
        let transition = self.chrome.osd.with_modal(|queue| {
            self.flow
                .dialogs
                .on_dismissed(id, queue, self.is_resumed, tick)
        });
        match transition {
            DismissTransition::Idle => {}
            DismissTransition::Reshow => {
                self.maybe_show_error_dialog(tick);
            }
            DismissTransition::ResumeAndReshow => {
                self.resume();
                let current_tick = self.core.session.tick_metrics();
                self.maybe_show_error_dialog(current_tick);
            }
        }
    }

    fn maybe_refresh_title(&mut self, now: Instant) {
        if self.core.shell.should_refresh_title(now)
            && let Some(window) = self.flow.render_sync.cloned_window()
        {
            window.set_title(
                &self
                    .core
                    .session
                    .window_title(&self.core.session.tick_metrics()),
            );
        }
    }

    fn exec(&mut self, cmd: SessionCommand) -> Option<SessionCommandOutcome> {
        match self.core.session.run_command(cmd) {
            Ok(o) => {
                if o.needs_redraw {
                    self.flow.render_sync.request_redraw(&mut self.core.shell);
                }
                Some(o)
            }
            Err(error) => {
                // Mobile has no stderr: a silent None looks like success.
                // The warn keeps the transport cause in logcat.
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
        self.chrome.peripherals.mark_dirty();
    }

    fn toggle_pause(&mut self) {
        self.exec(SessionCommand::TogglePause);
    }
}

impl SlotIo for AndroidFrontend {
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
}

impl SlotNav for AndroidFrontend {
    fn select_next_slot(&mut self) {
        self.exec(SessionCommand::SelectNextSlot);
    }

    fn select_previous_slot(&mut self) {
        self.exec(SessionCommand::SelectPreviousSlot);
    }

    fn select_slot(&mut self, slot_id: u64) {
        self.exec(SessionCommand::SelectActiveSlot(slot_id));
    }

    fn create_slot(&mut self) {
        self.exec(SessionCommand::CreateSlot);
    }
}

impl ResetControl for AndroidFrontend {
    fn reset(&mut self) {
        self.exec(SessionCommand::Reset);
    }
}

impl SessionControl for AndroidFrontend {
    fn unload(&mut self) {
        picker::reset_transient();
        self.discard_restore_state();
        if let Err(error) = self.core.session.unload() {
            log::warn!("ROM unload failed: {error}");
        }
    }

    fn apply_settings(
        &mut self,
        settings: SettingsSnapshot,
    ) -> Result<SettingsResult, SessionError> {
        let plan = self.core.session.apply_settings(settings)?;
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
        self.flow.lifecycle.begin_foreground_resume();
        self.try_resume_foreground(event_loop);
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        log::info!("ApplicationHandler::suspended");
        self.is_resumed = false;
        self.flow.lifecycle.cancel_foreground_resume();
        self.chrome
            .peripherals
            .stop(&self.app, &mut self.core.session);
        self.save_lifecycle_state();
        bridge::reset_transient();
        picker::reset_transient();
        settings::reset_transient();
        // Transient UI dies with the activity: release the gate, requeue the head.
        // The settings bridge pending id dies with its dialog.
        self.chrome
            .osd
            .with_modal(|queue| self.flow.dialogs.on_suspended(queue));
        self.release_window_resources();
    }

    fn window_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if !self.flow.render_sync.window_id_matches(window_id) {
            return;
        }

        match event {
            WindowEvent::CloseRequested | WindowEvent::Destroyed => {
                log::warn!("window_event: {event:?}");
                self.handle_surface_close();
            }
            WindowEvent::Focused(false) => {
                log::info!("window_event: focus lost");
                self.core.session.clear_input();
                self.chrome.input.forget_physical_pressed();
            }
            WindowEvent::Resized(size) => {
                log::info!("window_event: resized to {}x{}", size.width, size.height);
                self.flow
                    .render_sync
                    .resize_renderer(SurfaceSize::new(size.width, size.height));
                self.chrome.input.rebuild_overlay(
                    &self.core.session,
                    &self.app,
                    self.flow.render_sync.window_size(),
                );
                self.flow.render_sync.request_redraw(&mut self.core.shell);
            }
            WindowEvent::Touch(touch) => {
                self.chrome
                    .input
                    .handle_touch(&mut self.core.session, &self.app, touch);
            }
            WindowEvent::RedrawRequested => {
                // One build per pass: the bundle owns the session borrow
                // chain; render/Osd fan out from it without re-locking.
                let input = RenderSync::build_frame_input(&mut self.core.session);
                self.flow.render_sync.render(
                    &mut self.chrome.osd,
                    &mut self.core.shell,
                    Instant::now(),
                    &input,
                );
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if bridge::take_exit_request() {
            self.core.session.flush_before_exit();
            self.release_window_resources();
            event_loop.exit();
            return;
        }
        let now = Instant::now();
        // Each mutating handler receives fresh metrics, never a pre-drain tick.
        self.try_resume_foreground(event_loop);
        if let Some(result) = picker::take_result() {
            self.handle_picker_result(event_loop, result);
        }
        if self.is_resumed
            && let Some(result) = settings::take_result()
        {
            let tick = self.core.session.tick_metrics();
            self.handle_settings_result(result, tick);
        }
        bridge::take_events_into(&mut self.event_scratch);
        // Two-lane queue, no sort: priority acks (dialog dismissals)
        // drain first in arrival order, then the menu lane.
        let mut pending = std::mem::take(&mut self.event_scratch);
        for event in pending.drain(..) {
            let tick = self.core.session.tick_metrics();
            self.handle_menu_action(event, tick);
        }
        self.event_scratch = pending;
        // Re-read after mutation: the drain above may have loaded or
        // unloaded a ROM, so gates below never reuse the stale tick.
        let tick = self.core.session.tick_metrics();
        let is_resumed = self.is_resumed;
        self.chrome
            .peripherals
            .sync(&self.app, &mut self.core.session, is_resumed);
        self.maybe_refresh_title(now);
        // One snapshot borrow per pass: poll and the dialog gate share it.
        let (settings, language) = {
            let snapshot = self.core.session.settings_snapshot();
            (
                snapshot.local.notifications.clone(),
                snapshot.shared.general.language,
            )
        };
        if self
            .chrome
            .osd
            .poll(&mut self.core.session, &settings, language, now, tick)
        {
            self.flow.render_sync.request_redraw(&mut self.core.shell);
        }
        self.maybe_show_error_dialog(tick);

        if self.flow.render_sync.has_window()
            && self.core.shell.wants_redraw(tick.frame_counter)
            && let Some(window) = self.flow.render_sync.cloned_window()
        {
            window.request_redraw();
        }
        let deadline = crate::wake_deadline::next_deadline(
            self.flow.lifecycle.retry_deadline(),
            self.chrome.osd.wakeup(now),
        );
        event_loop.set_control_flow(deadline.map_or(ControlFlow::Wait, ControlFlow::WaitUntil));
    }
}

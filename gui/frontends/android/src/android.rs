mod bridge;
mod dialog_coordinator;
mod dialog_ownership;
mod error_dialog;
mod foreground_controller;
mod input_router;
mod lifecycle;
mod media_loader;
mod menu;
mod messages;
mod osd;
mod peripheral_sync;
mod picker;
mod render_sync;
mod saf;
mod settings;
mod storage;
mod touch_layout;

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
        access::{PauseControl, SessionControl, SettingsResult, SlotControl},
        commands::{SessionCommand, SessionCommandOutcome},
        notification::FailureOrigin,
        osd::TickMetrics,
    },
};
use nerust_render_traits::{SurfaceSize, renderer::GpuFactory};
use nerust_settings_core::model::shared::StoragePolicy;
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
    peripheral_sync::PeripheralSync,
    render_sync::RenderSync,
    settings::{AndroidSettings, SettingsApplier, SettingsPlan},
    storage::AndroidStorage,
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
                jni_str!("onErrorDialogDismissed"),
                jni_str!("()V"),
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
            bridge::call_on_activity(
                &init_fail_app,
                "finish Android activity after init failure",
                |env, activity| {
                    env.call_method(&activity, jni_str!("finish"), jni_sig!("()V"), &[])?;
                    Ok(())
                },
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
    peripherals: PeripheralSync,
    osd: osd::AndroidOsd,
    event_scratch: Vec<crate::android::messages::FrontendEvent>,
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
        gpu_factory: Rc<dyn GpuFactory>,
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
            peripherals: PeripheralSync::new(),
            osd: osd::AndroidOsd::new(),
            event_scratch: Vec::new(),
        };
        if frontend.lifecycle.restore_pending() {
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

    fn handle_picker_result(&mut self, event_loop: &ActiveEventLoop, result: RomPickerResult) {
        match result {
            RomPickerResult::Selected(uri) => {
                log::info!("handle_picker_result: picker returned URI; fetching off-thread");
                // Document/SAF I/O blocks: fetch on a worker thread and
                // dispatch the completed buffer when it publishes RomBytes.
                picker::spawn_document_fetch(self.app.clone(), uri);
            }
            RomPickerResult::RomBytes {
                uri,
                file_name,
                bytes,
            } => {
                log::info!(
                    "handle_picker_result: worker delivered {} bytes for {uri}",
                    bytes.len()
                );
                if let Err(error) = self.open_rom_from_bytes(event_loop, &uri, &file_name, bytes) {
                    log::error!("{error}");
                    self.session.report_external_error(
                        nerust_gui_shell::session::notification::FailureOrigin::Picker,
                        error,
                    );
                }
            }
            RomPickerResult::Failed(error) => {
                log::error!("{error}");
                self.session.report_external_error(
                    nerust_gui_shell::session::notification::FailureOrigin::Picker,
                    error,
                );
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
        // Settings closed: flush the error queue and resume an orphaned pause.
        // The gate reads DialogOwnership; no cached flag exists to go stale.
        let is_applied = matches!(result, SettingsDialogResult::Applied(_));
        if self.dialogs.is_settings_open() && !is_applied {
            // Stale synthesized dismiss racing a suspend-then-rerequest: ignore it.
            // The live flight's result arrives with its own request id.
            log::debug!("handle_settings_result: stale result while dialog open; ignoring");
            return;
        }
        if self.dialogs.is_settings_open() {
            // Applied racing a live re-request: the values are real user input, so apply them.
            // Error flush stays shielded below; the user re-saves if needed.
            log::warn!("handle_settings_result: applying settings while a newer dialog is open");
        } else {
            match self.osd.with_modal(|queue| {
                self.dialogs
                    .on_settings_closed(queue, self.is_resumed, tick)
            }) {
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
            self.session.settings_snapshot().clone(),
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
                // Push overlay and FPS-label settings to the live chrome.
                // Otherwise saved and visible settings diverge until the next load or resize.
                self.input.rebuild_overlay(
                    &self.session,
                    &self.app,
                    self.render_sync.window_size(),
                );
                self.render_sync.request_redraw(&mut self.shell);
                // Settings changed – refresh cached settings for sync dialogs.
                self.refresh_dialog_caches();
                // Peripheral settings may have changed: force a re-push
                // on the next pass (dirty generation).
                self.peripherals.mark_dirty();
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
        // Bridge truth, not the request outcome: the sync show path may leave this dialog up.
        // Every gate queries DialogOwnership instead.
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
                    // Rotation: rebuild the touch overlay even without a Resized event.
                    // Idempotent with the Resized path.
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

    /// Show the oldest queued error unless suspended, showing, or settings owns the screen.
    /// Takes the pass tick (one metrics lock per pass).
    fn maybe_show_error_dialog(&mut self, tick: TickMetrics) {
        let request = self
            .osd
            .with_modal(|queue| self.dialogs.poll_error_dialog(queue, self.is_resumed, tick));
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

    /// Dismiss hook: resumes only our own auto-pause, then shows the next queued error.
    /// `tick` is the pass tick; this handler must not take a second metrics lock.
    fn handle_error_dialog_dismissed(&mut self, tick: TickMetrics) {
        // Bridge truth here: a settings dialog may have opened over the
        // error since the last event.
        let transition = self
            .osd
            .with_modal(|queue| self.dialogs.on_dismissed(queue, self.is_resumed));
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
        self.peripherals.mark_dirty();
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
        self.peripherals.stop(&self.app, &mut self.session);
        self.save_lifecycle_state();
        bridge::reset_transient();
        picker::reset_transient();
        settings::reset_transient();
        // Transient UI dies with the activity: release the gate, requeue the head.
        // The settings bridge pending id dies with its dialog.
        self.osd
            .with_modal(|queue| self.dialogs.on_suspended(queue));
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
                // One build per pass: the bundle owns the session borrow
                // chain; render/Osd fan out from it without re-locking.
                let input = RenderSync::build_frame_input(&mut self.session);
                self.render_sync.render(
                    &self.app,
                    &mut self.osd,
                    &mut self.shell,
                    Instant::now(),
                    &input,
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
        let is_resumed = self.is_resumed;
        self.peripherals
            .sync(&self.app, &mut self.session, is_resumed);
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

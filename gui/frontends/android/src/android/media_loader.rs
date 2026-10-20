//! MediaLoader: ROM/document loading owned by one module.
//! Dispatches completed worker-thread fetch buffers.

use nerust_core_traits::factory::load::MediaObject;
use nerust_gui_shell::session::access::PauseControl as _;
use winit::event_loop::ActiveEventLoop;

use super::{bridge, storage::LastMediaReference};

impl super::AndroidFrontend {
    /// Pure apply for a library buffer fetched on a worker thread.
    /// Records the legacy digest after load for picker-vs-legacy checks.
    pub(super) fn open_library_from_bytes(
        &mut self,
        event_loop: &ActiveEventLoop,
        id: &str,
        bytes: Vec<u8>,
        digest: [u8; 32],
    ) -> Result<(), String> {
        log::info!(
            "open_library_from_bytes: applying id={id} ({} bytes)",
            bytes.len()
        );
        let path = self
            .core
            .storage
            .resolve_library_path(id)
            .ok_or_else(|| format!("ROM {id} was not found in the library"))?;
        let media = MediaObject::new(Some(path), bytes);
        self.load_media(event_loop, media, None, true)?;
        self.pending_legacy_digest = Some(digest);
        log::info!("open_library_from_bytes: legacy ROM restored without its document");
        Ok(())
    }

    /// Pure apply for a document buffer fetched on a worker thread.
    /// No I/O or hashing here; the event thread only dispatches.
    pub(super) fn apply_document_bytes(
        &mut self,
        event_loop: &ActiveEventLoop,
        reference: LastMediaReference,
        media: MediaObject,
        restore_hidden_state: bool,
    ) -> Result<(), String> {
        log::info!(
            "apply_document_bytes: applying '{}' from {} restore_hidden_state={restore_hidden_state}",
            reference.display_name,
            reference.uri
        );
        self.load_media(event_loop, media, Some(reference), restore_hidden_state)
    }

    pub(super) fn load_media(
        &mut self,
        event_loop: &ActiveEventLoop,
        media: MediaObject,
        document_reference: Option<LastMediaReference>,
        restore_hidden_state: bool,
    ) -> Result<(), String> {
        let (factory, system_id) = {
            let f = self
                .core
                .session
                .detect_factory(&media)
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

        self.core.session.clear_input();
        self.chrome
            .input
            .reset_touch_tracking(&mut self.core.session, &self.app);
        self.chrome.input.forget_physical_pressed();
        self.core
            .session
            .set_active_system(system_id.as_ref())
            .map_err(|e| format!("failed to activate system {system_id}: {e}"))?;

        let options = self
            .core
            .session
            .default_load_options()
            .ok_or_else(|| "no active system".to_string())?;
        let view = nerust_settings_core::factory::settings_view(
            self.core.session.settings_snapshot(),
            system_id.as_ref(),
        );
        let resolved = factory
            .resolve_load_request(&view, options)
            .map_err(|e| format!("failed to resolve ROM load request: {e}"))?;

        if let Err(error) = self.core.session.load_resolved(media, resolved) {
            return Err(format!("failed to start ROM: {error}"));
        }
        let restore_document_on_restart = document_reference.as_ref().is_some_and(|reference| {
            if let Err(error) = self.core.storage.save_last_media_reference(reference) {
                log::warn!("{error}");
                false
            } else {
                true
            }
        });
        self.finish_rom_load(event_loop, restore_hidden_state);
        if restore_document_on_restart {
            self.core.storage.touch_restore_pending();
        }
        self.notify_rom_loaded(&system_id.to_string());
        log::info!("load_media: session ready for system={system_id}");
        Ok(())
    }

    /// Tell Kotlin a ROM loaded. Fire-and-forget: the event thread
    /// never blocks on the notify round-trip.
    pub(super) fn notify_rom_loaded(&mut self, system_id: &str) {
        let system_id = system_id.to_string();
        bridge::post_to_activity(
            &self.app,
            "notify Android ROM load",
            move |env, activity| {
                let system_id = env.new_string(&system_id)?;
                bridge::call_bridge_method(
                    env,
                    &activity,
                    &bridge::methods::NOTIFY_ROM_LOADED,
                    &[jni::objects::JValue::Object(system_id.as_ref())],
                )?;
                Ok(())
            },
            None,
        );
    }

    pub(super) fn open_rom_from_bytes(
        &mut self,
        event_loop: &ActiveEventLoop,
        uri: &str,
        file_name: &str,
        bytes: Vec<u8>,
        digest: [u8; 32],
        restore_hidden_state: bool,
    ) -> Result<(), String> {
        log::info!(
            "open_rom_from_bytes: opening {uri} ({} bytes) restore_hidden_state={restore_hidden_state}",
            bytes.len()
        );
        // Carried from the fetch worker: no second hash on this thread.
        if let Some(expected) = self.pending_legacy_digest
            && expected != digest
        {
            return Err("selected ROM does not match the legacy library entry".to_string());
        }
        let media = MediaObject::from_document_uri(uri, file_name, bytes);
        let detected_system = self
            .core
            .session
            .detect_factory(&media)
            .map_err(|error| format!("failed to detect ROM system: {error}"))?
            .ok_or_else(|| "unsupported ROM format".to_string())?
            .system_id()
            .to_string();
        let reference =
            LastMediaReference::new(uri.to_string(), file_name.to_string(), detected_system);
        // Pure apply below: detection done, the event thread only dispatches.
        self.apply_document_bytes(event_loop, reference, media, restore_hidden_state)?;
        self.pending_legacy_digest = None;
        Ok(())
    }
    pub(super) fn finish_rom_load(
        &mut self,
        event_loop: &ActiveEventLoop,
        restore_hidden_state: bool,
    ) {
        if restore_hidden_state {
            log::info!("finish_rom_load: restoring hidden lifecycle state");
            self.core.session.load_hidden_lifecycle_state();
        } else {
            log::info!("finish_rom_load: clearing hidden lifecycle state");
            self.core.session.clear_hidden_lifecycle_state();
        }
        // Requeue the unacked head: stale batches describe the previous ROM.
        // The pause flag survives, holding resume until dismissal.
        self.chrome
            .osd
            .with_modal(|queue| self.flow.dialogs.on_suspended(queue));
        // Resume fresh loads and own auto-pauses, never across an error auto-pause.
        // Manual pauses survive.
        let policy = self.flow.lifecycle.policy(
            restore_hidden_state,
            self.flow.dialogs.auto_paused(),
            false,
            false,
        );
        if policy.should_resume() {
            self.flow.lifecycle.set_auto_paused(false);
            self.resume();
        }
        self.flow.lifecycle.clear_restore_pending();
        self.core.storage.clear_restore_pending();

        // Ensure a window and renderer exist immediately when resuming so that
        // request_redraw() takes effect without requiring a user tap.
        if !self.flow.render_sync.has_window() && self.is_resumed {
            match self.flow.render_sync.ensure_window(
                event_loop,
                &self.core.session,
                &self.core.gpu_factory,
                &mut self.chrome.osd,
                &mut self.core.shell,
            ) {
                Ok(()) => {
                    log::info!("finish_rom_load: ensured window/renderer after ROM load");
                    self.chrome.input.rebuild_overlay(
                        &self.core.session,
                        &self.app,
                        self.flow.render_sync.window_size(),
                    );
                }
                Err(error) => {
                    log::warn!(
                        "finish_rom_load: ensure_window failed: {error}; scheduling foreground resume"
                    );
                    self.flow.lifecycle.begin_foreground_resume();
                }
            }
        } else if let Some(window) = self.flow.render_sync.cloned_window() {
            // Rebuild renderer to ensure the underlying surface is fresh; dialogs or
            // system UI may have invalidated the previous surface.
            log::info!("finish_rom_load: rebuilding renderer to ensure visible surface");
            self.flow.render_sync.rebuild_renderer(
                window,
                &self.core.session,
                &self.core.gpu_factory,
                &mut self.chrome.osd,
                &mut self.core.shell,
            );
            self.chrome.input.rebuild_overlay(
                &self.core.session,
                &self.app,
                self.flow.render_sync.window_size(),
            );
        }

        self.refresh_dialog_caches();
        self.flow.render_sync.request_redraw(&mut self.core.shell);
    }
}

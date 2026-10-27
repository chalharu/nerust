//! Auxiliary lifecycle policy over the caller's generational native registry.
//! No additional dialog phase or settings draft is stored here.

use std::{hash::Hash, sync::Arc};

use nerust_core_traits::audio::AudioBackendRegistry;
use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_gui_shell::{registry::SystemRegistry, session::SessionError};

use super::ApplicationHost;
use crate::{
    aux::{AuxId, AuxWindows, OpenRequest},
    focus::AuxKind,
    session::SessionEvent,
    settings::{SettingsController, find_duplicate_system_id},
};

pub type WindowClose<H> = (
    Option<H>,
    Result<Option<nerust_gui_shell::session::access::SettingsResult>, SessionError>,
);

impl ApplicationHost {
    /// Reserve before any native work. Existing/pending requests never acquire
    /// settings pause again. A rejected session transition leaves no reservation.
    pub fn reserve_window<N: Copy + Eq + Hash, H>(
        &mut self,
        windows: &mut AuxWindows<N, H>,
        kind: AuxKind,
    ) -> Result<Option<OpenRequest>, SessionError> {
        if self.close_exited() {
            return Ok(None);
        }
        let request = windows.request_open(kind);
        if kind == AuxKind::Settings
            && let OpenRequest::Create(id) = request
        {
            match self.dispatch(SessionEvent::SettingsOpened) {
                Ok(outcome) if outcome.settings_changed => (),
                Ok(_) => {
                    windows.cancel_open(id);
                    return Ok(None);
                }
                Err(error) => {
                    windows.cancel_open(id);
                    return Err(error);
                }
            }
        }
        Ok(Some(request))
    }

    /// Build exactly one controller and publish its actual initial projection.
    /// Validation and rollback are shared by all native adapters.
    pub fn prepare_settings_window<N: Copy + Eq + Hash, H>(
        &mut self,
        windows: &mut AuxWindows<N, H>,
        id: AuxId,
        registry: Arc<SystemRegistry>,
        audio: Arc<AudioBackendRegistry>,
    ) -> Result<Option<SettingsController>, String> {
        if windows.kind_of_id(id) != Some(AuxKind::Settings)
            || windows.get(AuxKind::Settings).is_some()
        {
            return Ok(None);
        }
        if let Some(duplicate) = find_duplicate_system_id(&registry) {
            self.cancel_window(windows, id);
            return Err(format!("duplicate SystemId '{duplicate}'"));
        }
        let controller = SettingsController::new(self.settings_snapshot(), registry, audio);
        *windows.window_mut(id).expect("current reservation") = controller.window();
        Ok(Some(controller))
    }

    /// A stale cancellation must not release a newer settings owner's pause.
    pub fn cancel_window<N: Copy + Eq + Hash, H>(
        &mut self,
        windows: &mut AuxWindows<N, H>,
        id: AuxId,
    ) -> bool {
        let kind = windows.kind_of_id(id);
        if !windows.cancel_open(id) {
            return false;
        }
        if kind == Some(AuxKind::Settings) {
            let _ = self.dispatch(SessionEvent::SettingsOpenFailed);
        }
        true
    }

    /// Apply before releasing settings pause, including on an apply error.
    /// Native scaling/fullscreen/chrome effects consume the returned result.
    pub fn finish_settings(
        &mut self,
        pending: Option<SettingsSnapshot>,
    ) -> Result<Option<nerust_gui_shell::session::access::SettingsResult>, SessionError> {
        let result = pending
            .map(|snapshot| self.apply_settings(snapshot))
            .transpose();
        self.settings_closed();
        result
    }

    pub fn settings_closed(&mut self) -> bool {
        self.dispatch(SessionEvent::SettingsClosed)
            .is_ok_and(|outcome| outcome.settings_changed)
    }

    /// Validate callback generation before taking a handle or applying a draft.
    /// Returned native resources must be dropped outside toolkit owner borrows.
    pub fn close_window<N: Copy + Eq + Hash, H>(
        &mut self,
        windows: &mut AuxWindows<N, H>,
        id: AuxId,
        pending: Option<SettingsSnapshot>,
    ) -> Option<WindowClose<H>> {
        let kind = windows.kind_of_id(id)?;
        let handle = windows.remove_id(id);
        let result = if kind == AuxKind::Settings {
            self.finish_settings(pending)
        } else {
            if kind == AuxKind::History {
                self.invalidate_history();
            }
            Ok(None)
        };
        Some((handle, result))
    }
}

#[cfg(test)]
mod tests;

//! Auxiliary lifecycle policy over the caller's generational native registry.
//! No additional dialog phase or settings draft is stored here.

use std::hash::Hash;

use nerust_gui_shell::session::SessionError;

use super::ApplicationHost;
use crate::{
    aux::{AuxId, AuxWindows, OpenRequest},
    focus::AuxKind,
    session::SessionEvent,
    widget_session::{Pending, WidgetSession, WindowSubmission},
    window::WindowEffects,
};
use nerust_frontend_viewdefs::settings::{SettingsController, find_duplicate_system_id};

pub type WidgetWindowClose<H> = (Option<H>, Result<WindowEffects, SessionError>);

impl ApplicationHost {
    /// Prepare the generic control boundary for both Settings and History.
    /// Registries come from the owned session: native adapters never pass
    /// shell or audio types across the boundary.
    pub fn prepare_widget_window<N: Copy + Eq + Hash, H>(
        &mut self,
        windows: &mut AuxWindows<N, H>,
        id: AuxId,
    ) -> Result<Option<WidgetSession>, String> {
        if self.close_exited() || windows.mailbox(id).is_some() {
            return Ok(None);
        }
        let session = match windows.kind_of_id(id) {
            Some(AuxKind::Settings) => {
                let Some(controller) = self.prepare_settings_window(windows, id)? else {
                    return Ok(None);
                };
                WidgetSession::from_settings(controller)
            }
            Some(AuxKind::History) if windows.get(AuxKind::History).is_none() => {
                let language = self.settings_snapshot().shared.general.language;
                let lines = self
                    .session
                    .notification_history_lines_cached(language)
                    .into();
                WidgetSession::history(lines, language)
            }
            _ => return Ok(None),
        };
        *windows.window_mut(id).expect("current reservation") = session.window();
        windows.set_mailbox(id, session.mailbox());
        Ok(Some(session))
    }

    /// Refresh History from real session data (also after clear or language
    /// changes). Settings keeps its one draft and is intentionally not reset.
    pub fn refresh_widget_session(&mut self, session: &mut WidgetSession) {
        if session.kind() == AuxKind::History {
            let language = self.settings_snapshot().shared.general.language;
            let lines = self
                .session
                .notification_history_lines_cached(language)
                .into();
            session.refresh_history(lines, language);
        }
    }

    /// GTK-style apply before native destruction. Failure clears close intent,
    /// publishes an error and preserves the same draft, registry and pause owner.
    /// A clone of a consumed submission cannot apply it (or a future edit) twice.
    pub fn apply_window_submission<N: Copy + Eq + Hash, H>(
        &mut self,
        windows: &AuxWindows<N, H>,
        id: AuxId,
        submission: WindowSubmission,
    ) -> Result<WindowEffects, SessionError> {
        if self.close_exited()
            || !windows
                .mailbox(id)
                .is_some_and(|m| m.same(&submission.mailbox))
        {
            return Ok(WindowEffects::default());
        }
        self.consume_window_submission(submission)
    }

    fn consume_window_submission(
        &mut self,
        submission: WindowSubmission,
    ) -> Result<WindowEffects, SessionError> {
        match submission.take() {
            Some(Pending::Settings(snapshot)) => match self.apply_settings(*snapshot) {
                Ok(result) => {
                    let fullscreen = result.fullscreen_default_changed.then_some(
                        self.settings_snapshot()
                            .local
                            .video
                            .window
                            .fullscreen_default,
                    );
                    let resize = if result.scaling_changed {
                        nerust_gui_shell::settings::scaling_factor(
                            self.settings_snapshot().local.video.window.scaling,
                        )
                        .map(|scale| {
                            let size = self.session.window_size();
                            (
                                (size.width * scale as f32).round().max(1.0) as u32,
                                (size.height * scale as f32).round().max(1.0) as u32,
                            )
                        })
                    } else {
                        None
                    };
                    self.note_redraw();
                    Ok(WindowEffects {
                        rebuild_renderer: result.renderer_needs_rebuild,
                        fullscreen,
                        resize,
                        redraw: true,
                        refresh_chrome: true,
                    })
                }
                Err(error) => {
                    submission.mailbox.apply_failed(error.to_string());
                    Err(error)
                }
            },
            Some(Pending::ClearHistory) => {
                self.clear_history();
                self.invalidate_history();
                submission.mailbox.invalidate();
                self.note_redraw();
                Ok(WindowEffects {
                    redraw: true,
                    ..Default::default()
                })
            }
            None => Ok(WindowEffects::default()),
        }
    }

    /// Generation-checked native close. Tao may take/remove the native handle
    /// before applying; GTK calls apply_window_submission first and only closes
    /// on success. Resources returned here must be dropped outside owner borrows.
    /// No new pause-phase state: the existing Settings owner remains authoritative.
    pub fn close_widget_window<N: Copy + Eq + Hash, H>(
        &mut self,
        windows: &mut AuxWindows<N, H>,
        id: AuxId,
        pending: Option<WindowSubmission>,
    ) -> Option<WidgetWindowClose<H>> {
        let kind = windows.kind_of_id(id)?;
        if let Some(submission) = &pending
            && !windows
                .mailbox(id)
                .is_some_and(|m| m.same(&submission.mailbox))
        {
            return None;
        }
        let handle = windows.remove_id(id);
        let result = if self.close_exited() {
            Ok(WindowEffects::default())
        } else {
            pending
                .map(|submission| self.consume_window_submission(submission))
                .unwrap_or_else(|| Ok(WindowEffects::default()))
        };
        if kind == AuxKind::Settings {
            self.settings_closed();
        }
        if kind == AuxKind::History {
            self.invalidate_history();
        }
        Some((
            handle,
            result.map(|mut effects| {
                effects.refresh_chrome = true;
                effects
            }),
        ))
    }

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

    /// Build exactly one settings draft; the caller publishes its generic projection.
    /// Validation and rollback are shared by all native adapters.
    fn prepare_settings_window<N: Copy + Eq + Hash, H>(
        &mut self,
        windows: &mut AuxWindows<N, H>,
        id: AuxId,
    ) -> Result<Option<SettingsController>, String> {
        if windows.kind_of_id(id) != Some(AuxKind::Settings)
            || windows.get(AuxKind::Settings).is_some()
        {
            return Ok(None);
        }
        let registry = self.session.system_registry().clone();
        if let Some(duplicate) = find_duplicate_system_id(&registry) {
            self.cancel_window(windows, id);
            return Err(format!("duplicate SystemId '{duplicate}'"));
        }
        let audio = self.session.audio_registry().clone();
        let controller = SettingsController::new(self.settings_snapshot(), registry, audio);
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

    /// Also used by native lifecycle tests after registry removal.
    pub fn settings_closed(&mut self) -> bool {
        self.dispatch(SessionEvent::SettingsClosed)
            .is_ok_and(|outcome| outcome.settings_changed)
    }
}

#[cfg(test)]
mod tests;

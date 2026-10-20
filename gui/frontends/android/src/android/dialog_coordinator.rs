//! DialogCoordinator: frontend-owned dialog flow, no OSD pass-through.
//!
//! Previously `AndroidOsd` wrapped the `ErrorDialogController` behind five
//! delegating methods while also owning raster/strip/insets state. The
//! coordinator owns the controller directly on the frontend; `AndroidOsd`
//! keeps raster/strip/insets (plus the notification manager, borrowed per
//! call) only. Screen gates come from [`DialogOwnership`] (bridge truth),
//! never from a cached flag.

use nerust_gui_shell::session::osd::{ModalSink, ModalSource, TickMetrics};

use super::dialog_ownership::DialogOwnership;
use crate::error_dialog_controller::{
    DismissTransition, Screen, SettingsClosedTransition, ShowTransition,
};

/// Frontend-owned error-dialog flow. The JNI handoff reads the showing
/// batch via [`DialogCoordinator::current_detail`].
pub(super) struct DialogCoordinator {
    controller: crate::error_dialog_controller::ErrorDialogController,
    ownership: DialogOwnership,
}

impl DialogCoordinator {
    pub(super) fn new(ownership: DialogOwnership) -> Self {
        Self {
            controller: crate::error_dialog_controller::ErrorDialogController::new(),
            ownership,
        }
    }

    /// Borrow the showing batch for the JNI handoff.
    pub(super) fn current_detail(&self) -> Option<&str> {
        self.controller.current_detail()
    }

    /// Whether the settings dialog is up (bridge truth via ownership).
    /// Replaces the deleted cached bool: every gate queries this.
    pub(super) fn is_settings_open(&self) -> bool {
        self.ownership.is_settings_open()
    }

    /// Whether the session is paused on behalf of the error dialog.
    pub(super) fn auto_paused(&self) -> bool {
        self.controller.auto_paused()
    }

    fn screen(&self, is_resumed: bool, tick: TickMetrics) -> Screen {
        Screen::show_gate(
            is_resumed,
            self.ownership.is_settings_open(),
            tick.can_pause(),
        )
    }

    /// Show the oldest queued error unless suspended, already showing,
    /// or settings owns the screen. Takes the pass tick so the caller
    /// pays one metrics lock, not one per dialog gate.
    pub(super) fn poll_error_dialog(
        &mut self,
        queue: &mut (impl ModalSink + ModalSource),
        is_resumed: bool,
        tick: TickMetrics,
    ) -> Option<ShowTransition> {
        self.controller
            .poll_show(queue, self.screen(is_resumed, tick))
    }

    /// Dismiss hook from the Kotlin dialog. Resumes only our own
    /// auto-pause, then asks for the next batch.
    pub(super) fn on_dismissed(
        &mut self,
        queue: &mut (impl ModalSink + ModalSource),
        is_resumed: bool,
    ) -> DismissTransition {
        // Bridge truth here: a settings dialog may have opened over the
        // error between the cached flag's last event-driven update.
        let screen = Screen::show_gate(is_resumed, self.ownership.is_settings_open(), false);
        self.controller.on_dismissed(queue, screen)
    }

    /// Activity suspend kills transient UI: release the gate and requeue
    /// the unacked head; the pause flag survives for the resume path.
    pub(super) fn on_suspended(&mut self, queue: &mut (impl ModalSink + ModalSource)) {
        self.controller.on_suspended(queue);
    }

    /// Settings close flushes the queue; an orphaned auto-pause resumes
    /// only when no dialog came up to own it.
    pub(super) fn on_settings_closed(
        &mut self,
        queue: &mut (impl ModalSink + ModalSource),
        is_resumed: bool,
        tick: TickMetrics,
    ) -> SettingsClosedTransition {
        let screen = Screen::show_gate(
            is_resumed,
            self.ownership.is_settings_open(),
            tick.can_pause(),
        );
        self.controller.on_settings_closed(queue, screen)
    }
}

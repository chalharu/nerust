//! DialogCoordinator: frontend-owned dialog flow, no OSD pass-through.
//! Screen gates come from the injected [`ScreenSource`]; no cached bool.

use nerust_gui_shell::session::osd::{ModalSink, ModalSource, TickMetrics};

use crate::error_dialog_controller::{
    DismissTransition, Screen, SettingsClosedTransition, ShowTransition,
};

/// Source of screen truth for dialog gates; injected for isolation.
pub(super) trait ScreenSource {
    /// Whether the settings dialog is currently up (bridge truth).
    fn is_settings_open(&self) -> bool;
}

/// Frontend-owned error-dialog flow. The JNI handoff reads the showing
/// batch via [`DialogCoordinator::current_detail`].
pub(super) struct DialogCoordinator<S> {
    controller: crate::error_dialog_controller::ErrorDialogController,
    source: S,
}

impl<S: ScreenSource> DialogCoordinator<S> {
    pub(super) fn new(source: S) -> Self {
        Self {
            controller: crate::error_dialog_controller::ErrorDialogController::new(),
            source,
        }
    }

    /// Borrow the showing batch for the JNI handoff.
    pub(super) fn current_detail(&self) -> Option<&str> {
        self.controller.current_detail()
    }

    /// Whether the settings dialog is up (source truth, never cached).
    pub(super) fn is_settings_open(&self) -> bool {
        self.source.is_settings_open()
    }

    /// Whether the session is paused on behalf of the error dialog.
    pub(super) fn auto_paused(&self) -> bool {
        self.controller.auto_paused()
    }

    /// Single gate ctor for both lanes: every dialog decision
    /// routes through `Screen::show_gate` with the live tick.
    fn screen(&self, is_resumed: bool, tick: TickMetrics) -> Screen {
        Screen::show_gate(is_resumed, self.source.is_settings_open(), tick.can_pause())
    }

    /// Show the oldest queued error unless suspended, showing,
    /// or settings owns the screen. Takes the pass tick.
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
    /// auto-pause, then asks for the next batch. Takes the pass tick.
    pub(super) fn on_dismissed(
        &mut self,
        queue: &mut (impl ModalSink + ModalSource),
        is_resumed: bool,
        tick: TickMetrics,
    ) -> DismissTransition {
        self.controller
            .on_dismissed(queue, self.screen(is_resumed, tick))
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
        self.controller
            .on_settings_closed(queue, self.screen(is_resumed, tick))
    }
}

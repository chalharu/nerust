//! Native auxiliary creation and settings effects. Policy lives view-side;
//! native code routes handles, screens, and mailboxes through the app trait.
use super::HostState;
use nerust_frontend_host::{
    aux::{AuxId, OpenRequest},
    focus::AuxKind,
};

use nerust_frontend_host::window::WindowEffects;
use wakui::control::ControlEffect;

use crate::ports::{AuxOpenParams, WindowOpener};

impl HostState {
    pub(crate) fn reserve_aux_window(&mut self, kind: AuxKind) -> Option<OpenRequest> {
        match self.core.reserve_widget(&mut self.aux.windows, kind) {
            Ok(request) => request,
            Err(error) => {
                log::warn!("auxiliary reservation failed: {error}");
                None
            }
        }
    }

    /// One native creation/completion path for every common auxiliary Window.
    pub(crate) fn create_aux_window(
        &mut self,
        id: AuxId,
        opener: &dyn WindowOpener,
        params: AuxOpenParams,
    ) -> bool {
        let Some(kind) = self.aux.windows.kind_of_id(id) else {
            return false;
        };
        if self.aux.windows.get(kind).is_some() {
            return false;
        }
        let window = self.aux.window(id).expect("current reservation").clone();
        let opened = opener
            .create_aux(window, params)
            .is_some_and(|handle| self.aux.complete_open(id, handle).is_ok());
        if !opened {
            self.core.cancel_widget(&mut self.aux.windows, id);
        }
        opened
    }

    pub(crate) fn apply_window_effects(&mut self, effects: WindowEffects) -> bool {
        if let Some(fullscreen) = effects.fullscreen
            && let Some(window) = self.windows.live()
        {
            window.set_fullscreen(super::fullscreen_mode(fullscreen));
        }
        if let Some((width, height)) = effects.resize
            && let Some(window) = self.windows.live()
        {
            let extra = self.windows.menu_height();
            window.set_inner_size(tao::dpi::LogicalSize::new(
                f64::from(width),
                f64::from(height) + extra,
            ));
        }
        if effects.refresh_chrome {
            self.sync_chrome(effects.redraw);
        } else if effects.redraw {
            self.request_redraw();
        }
        effects.rebuild_renderer
    }

    pub(super) fn open_settings_window(&mut self, opener: &dyn WindowOpener) {
        let Some(OpenRequest::Create(id)) = self.reserve_aux_window(AuxKind::Settings) else {
            return;
        };
        if !self.prepare_widget(id) {
            return;
        }
        let (screen, mailbox) = match (self.core.widget_screen(id), self.core.widget_mailbox(id)) {
            (Some(screen), Some(mailbox)) => (screen, mailbox),
            _ => {
                self.core.cancel_widget(&mut self.aux.windows, id);
                return;
            }
        };
        let opened = self.create_aux_window(
            id,
            opener,
            AuxOpenParams::Controls {
                id,
                screen: Box::new(screen),
                mailbox,
            },
        );
        if !opened {
            log::error!("failed to open settings window");
            self.sync_chrome(false);
        } else {
            self.sync_menu_state();
        }
    }

    pub(crate) fn prepare_widget(&mut self, id: AuxId) -> bool {
        match self.core.prepare_widget(&mut self.aux.windows, id) {
            Ok(prepared) => prepared,
            Err(error) => {
                log::error!("refusing settings window: {error}");
                self.sync_chrome(false);
                false
            }
        }
    }

    /// Pump one widget turn: drain queued control events through the app,
    /// run picker dialogs natively, then push the latest screen.
    pub(crate) fn pump_widgets(&mut self) {
        let ids: Vec<AuxId> = [AuxKind::Settings, AuxKind::History]
            .into_iter()
            .filter_map(|kind| self.aux.route_id(kind))
            .collect();
        for id in ids {
            for event in self.aux.drain_widget_events(id) {
                for effect in self.core.widget_dispatch(id, event) {
                    self.apply_control_effect(id, effect);
                }
            }
            let kind = self.aux.windows.kind_of_id(id);
            if kind == Some(AuxKind::History) && self.core.history_changed() {
                self.core.widget_refresh(id);
            }
            if let Some(screen) = self.core.widget_screen(id) {
                self.aux.push_widget_snapshot(id, &screen);
            }
        }
    }

    fn apply_control_effect(&mut self, id: AuxId, effect: ControlEffect) {
        let ControlEffect::PickDirectory {
            title,
            target,
            request,
        } = effect;
        let path = rfd::FileDialog::new().set_title(title).pick_folder();
        self.core.widget_submit_picker(id, target, request, path);
        if let Some(screen) = self.core.widget_screen(id) {
            self.aux.push_widget_snapshot(id, &screen);
        }
    }

    /// The caller retains the original generation across native dispatch.
    pub(crate) fn finish_widget_window(&mut self, id: AuxId, close: bool) -> bool {
        if self.aux.windows.kind_of_id(id).is_none() {
            return false;
        }
        let Some(outcome) = self.core.close_widget(&mut self.aux.windows, id, close) else {
            return false;
        };
        // The native handle dies with the window; effects drive chrome.
        let _native = outcome.native;
        self.apply_window_effects(outcome.effects)
    }
}

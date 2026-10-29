//! Native auxiliary windows: one generic Controls dialog per aux kind.
//! Opening, presence, identity, and routing; policy lives view-side.
use std::rc::Rc;

use crate::chrome::dialogs::{RenderedDialog, render_dialog};
use crate::chrome::surface_state::surface_gone;
use crate::chrome::window::{Window, WindowExtend};
use crate::scene::binding::{Callbacks, WidgetDialog};
use gtk::prelude::*;
use wakui_core::{
    chrome::aux::{AuxId, OpenRequest},
    chrome::window::WindowEffects,
};

/// Native resources only. Opening, presence, identity and routing belong to host.
/// Every aux window is a generic Controls dialog (any aux kind).
#[derive(Clone)]
pub(crate) enum NativeAux {
    Controls(WidgetDialog),
}

impl NativeAux {
    pub(crate) fn is_gone(&self) -> bool {
        match self {
            Self::Controls(program) => surface_gone(&program.dialog),
        }
    }

    pub(crate) fn present(&self) {
        match self {
            Self::Controls(program) => {
                program.dialog.present();
                program.binding.sync();
            }
        }
    }

    pub(crate) fn native_window(&self) -> gtk::Window {
        match self {
            Self::Controls(program) => program.dialog.clone().upcast(),
        }
    }
}

/// Native liveness is an input; the shared registry decides create/present/pending.
pub(crate) fn request_aux<K: Copy + Eq + std::hash::Hash + 'static>(
    window: &Window<K>,
    kind: K,
) -> Option<AuxId> {
    let existing = {
        let window = window.borrow();
        window.aux.route_id(kind).zip(window.aux.get(kind).cloned())
    };
    if let Some((id, handle)) = existing
        && handle.is_gone()
    {
        close_aux(window, id);
    }
    let request = {
        let mut core = window.borrow_mut();
        let state = core.state.clone();
        let request = state.borrow_mut().host.reserve_widget(&mut core.aux, kind);
        match request {
            Ok(Some(request)) => request,
            Ok(None) => return None,
            Err(error) => {
                log::warn!("GTK auxiliary reservation failed: {error}");
                return None;
            }
        }
    };
    match request {
        OpenRequest::Create(id) => Some(id),
        OpenRequest::Present(_) => {
            let existing = window.borrow().aux.get(kind).cloned();
            if let Some(existing) = existing {
                existing.present();
            }
            None
        }
        OpenRequest::Pending(_) => None,
    }
}

pub(crate) fn complete_aux<K: Copy + Eq + std::hash::Hash + 'static>(
    window: &Window<K>,
    id: AuxId,
    handle: NativeAux,
) -> bool {
    let native = handle.native_window();
    let result = window.borrow_mut().aux.complete_open(id, None, handle);
    if let Err(rejected) = result {
        rejected.native_window().destroy();
        close_aux(window, id);
        return false;
    }
    let weak = Rc::downgrade(window);
    native.connect_destroy(move |_| {
        if let Some(window) = weak.upgrade() {
            close_aux(&window, id);
        }
    });
    let weak = Rc::downgrade(window);
    native.connect_unrealize(move |_| {
        if let Some(window) = weak.upgrade() {
            close_aux(&window, id);
        }
    });
    true
}

/// Remove before any native drop or session callback: both can synchronously reenter.
pub(crate) fn close_aux<K: Copy + Eq + std::hash::Hash + 'static>(
    window: &Window<K>,
    id: AuxId,
) -> bool {
    let kind = window.borrow().aux.kind_of_id(id);
    let outcome = {
        let mut core = window.borrow_mut();
        let state = core.state.clone();
        state
            .borrow_mut()
            .host
            .close_widget(&mut core.aux, id, true)
    };
    let Some(outcome) = outcome else {
        return false;
    };
    if let Some(kind) = kind {
        window
            .state()
            .borrow_mut()
            .host
            .remember_window_pos(kind, 0, 0);
    }
    // The native handle dies with the window; effects already applied.
    drop(outcome);
    true
}

pub(crate) fn open_auxiliary<K: Copy + Eq + std::hash::Hash + 'static>(
    window: &Window<K>,
    kind: K,
) {
    let Some(id) = request_aux(window, kind) else {
        return;
    };
    window.update_actions();
    // Host prepares the only draft and publishes its Window into the reserved
    // route. Validation/rollback and SettingsOpened/Closed are not native policy.
    let prepared = {
        let mut core = window.borrow_mut();
        let state = core.state.clone();
        state.borrow_mut().host.prepare_widget(&mut core.aux, id)
    };
    match prepared {
        Ok(true) => (),
        Ok(false) => return,
        Err(error) => {
            window.state().borrow_mut().host.report_host_error(error);
            window.update_actions();
            return;
        }
    }
    let (model, screen, mailbox) = {
        let core = window.borrow();
        let state = core.state.clone();
        let borrowed = state.borrow();
        (
            core.aux.window(id).cloned(),
            borrowed.host.widget_screen(id),
            borrowed.host.widget_mailbox(id),
        )
    };
    let (Some(model), Some(screen), Some(mailbox)) = (model, screen, mailbox) else {
        close_aux(window, id);
        return;
    };
    let built = render_dialog(
        &model,
        &window.window(),
        id,
        screen,
        mailbox,
        widget_callbacks(window, id),
    );
    let handle = match built {
        Some(RenderedDialog::Controls(built)) => NativeAux::Controls(built),
        None => {
            close_aux(window, id);
            window.update_actions();
            return;
        }
    };
    // Window position memory: view-side owns the kind; GTK4 cannot move
    // toplevels, but the read shows the wiring.
    let _ = window.state().borrow().host.window_pos(kind);
    let dialog_window = handle.native_window();
    // Aux focus: report arrivals/departures view-side by registry id.
    {
        let weak = Rc::downgrade(window);
        dialog_window.connect_is_active_notify(move |native| {
            if let Some(window) = weak.upgrade() {
                window
                    .state()
                    .borrow_mut()
                    .host
                    .drive_aux_focus(id, native.is_active());
            }
        });
    }
    let native = handle.native_window();
    if complete_aux(window, id, handle) {
        native.present();
    }
}

/// Drain deferred aux opens (e.g. one aux view requested from another
/// gesture) through the normal reserve/prepare path.
pub(crate) fn drain_deferred_opens<K: Copy + Eq + std::hash::Hash + 'static>(window: &Window<K>) {
    let kinds = window.state().borrow_mut().host.drain_open_requests();
    for kind in kinds {
        open_auxiliary(window, kind);
    }
}

/// Per-pump polled-widget refresh without kinds: the widget screen is the only
/// native view; the view-side poll behind it drives background work and ticks.
pub(crate) fn pump_debug_widgets<K: Copy + Eq + std::hash::Hash + 'static>(window: &Window<K>) {
    let id = match window.state().borrow().host.polled_widget() {
        Some(id) => id,
        None => return,
    };
    window.state().borrow_mut().host.widget_refresh(id);
    if let Some(program) = window
        .borrow()
        .aux
        .kind_of_id(id)
        .and_then(|kind| window.borrow().aux.get(kind).cloned())
    {
        match program {
            NativeAux::Controls(program) => program.binding.refresh(),
        }
    }
    let _ = window.state().borrow_mut().host.poll_widget(id);
    window.state().borrow_mut().host.widget_refresh(id);
    if let Some(program) = window
        .borrow()
        .aux
        .kind_of_id(id)
        .and_then(|kind| window.borrow().aux.get(kind).cloned())
    {
        match program {
            NativeAux::Controls(program) => program.binding.refresh(),
        }
    }
    window.state().borrow_mut().host.tick_widget(id);
}

pub(crate) fn widget_callbacks<K: Copy + Eq + std::hash::Hash + 'static>(
    window: &Window<K>,
    id: AuxId,
) -> Callbacks {
    let current = Rc::downgrade(window);
    let refresh = current.clone();
    let submit = current.clone();
    let picker = current.clone();
    let close = current.clone();
    Callbacks {
        current: Box::new(move || {
            current.upgrade().is_some_and(|window| {
                let core = window.borrow();
                core.aux.kind_of_id(id).is_some() && !core.state.borrow().host.close_exited()
            })
        }),
        refresh: Box::new(move || {
            let window = refresh.upgrade()?;
            let kind = window.borrow().aux.kind_of_id(id)?;
            let state = window.borrow().state.clone();
            let _ = kind;
            // Refresh the view-side session (History/PPU/Debug; no-op for
            // Settings) before reading the screen, so retained dialogs track
            // the host through the periodic sync timer.
            state.borrow_mut().host.widget_refresh(id);
            state.borrow().host.widget_screen(id)
        }),
        submit: Box::new(move |events| {
            let Some(window) = submit.upgrade() else {
                return Vec::new();
            };
            // Complete every owner borrow before running native effects:
            // applying chrome or actions reenters the window borrow.
            let (effects, outcome, deferred) = {
                let state = window.state();
                let mut borrowed = state.borrow_mut();
                let mut effects = Vec::new();
                for event in events {
                    effects.extend(borrowed.host.widget_dispatch(id, event));
                }
                // Consume a staged submission without closing: apply runs
                // after the borrows end; native teardown stays on the close path.
                let outcome = {
                    let mut core = window.borrow_mut();
                    borrowed.host.close_widget(&mut core.aux, id, false)
                };
                let deferred = borrowed.host.drain_open_requests();
                (effects, outcome, deferred)
            };
            if let Some(outcome) = outcome {
                apply_window_effects(&window, outcome.effects);
                window.update_actions();
            }
            for kind in deferred {
                open_auxiliary(&window, kind);
            }
            effects
        }),
        submit_picker: Box::new(move |target, request, path| {
            if let Some(window) = picker.upgrade() {
                let state = window.state();
                state
                    .borrow_mut()
                    .host
                    .widget_submit_picker(id, target, request, path);
            }
        }),
        close: Box::new(move || {
            if let Some(window) = close.upgrade()
                && close_aux(&window, id)
            {
                window.update_actions();
            }
        }),
    }
}

pub(crate) fn apply_window_effects<K: Copy + Eq + std::hash::Hash + 'static>(
    window: &Window<K>,
    effects: WindowEffects,
) {
    if effects.rebuild_renderer {
        window.state().borrow_mut().renderer_reload_pending = true;
    }
    let native = window.window();
    if let Some(fullscreen) = effects.fullscreen {
        native.set_fullscreened(fullscreen);
    }
    if let Some((width, height)) = effects.resize {
        native.set_default_size(width as i32, height as i32);
        native.queue_resize();
    }
    if effects.redraw {
        native.queue_draw();
    }
    if effects.refresh_chrome {
        window.update_actions();
    }
}

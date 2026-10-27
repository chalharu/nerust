//! Native lifetime and effects; all settings presentation comes from host.
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    rc::{Rc, Weak},
};

use gtk::{glib, prelude::*};
use nerust_frontend_host::settings::{
    ControlInput, SettingsControlId, SettingsController, SettingsEffect, SettingsEvent,
};
use nerust_frontend_host::window::Window as HostWindow;
use nerust_gui_shell::session::access::SessionControl;
use nerust_gui_viewmodel::settings::Subscription;
use nerust_settings_core::model::local::ScalingMode;

use crate::State;

#[path = "settings/controls.rs"]
mod controls;
use controls::{Registry, SettingsGtkVisitor};

/// Also restores the previous state for nested guards and unwinding callbacks.
struct FlagGuard<'a>(&'a Cell<bool>, bool);

impl<'a> FlagGuard<'a> {
    fn enter(flag: &'a Cell<bool>) -> Self {
        Self(flag, flag.replace(true))
    }
}

impl Drop for FlagGuard<'_> {
    fn drop(&mut self) {
        self.0.set(self.1);
    }
}

pub(crate) struct SettingsBinding {
    self_weak: Weak<Self>,
    controller: RefCell<SettingsController>,
    _subscription: Subscription,
    dialog: glib::WeakRef<gtk::Dialog>,
    stack: gtk::Stack,
    error_label: gtk::Label,
    registry: RefCell<Registry>,
    refreshing: Cell<bool>,
    updating: Cell<bool>,
    pending: Cell<bool>,
    scheduled: Cell<bool>,
    draining: Cell<bool>,
    events: RefCell<VecDeque<SettingsEvent>>,
    chooser: RefCell<Option<gtk::FileChooserNative>>,
    closed: Cell<bool>,
}

impl SettingsBinding {
    pub(crate) fn dialog(&self) -> Option<gtk::Dialog> {
        self.dialog.upgrade()
    }

    pub(crate) fn render(&self, screen: &nerust_frontend_host::settings::SettingsScreen) {
        screen.visit(&mut SettingsGtkVisitor::new(self));
    }

    fn new(controller: SettingsController, dialog: &gtk::Dialog) -> Rc<Self> {
        let stack = gtk::Stack::new();
        stack.set_hexpand(true);
        stack.set_vexpand(true);
        let sidebar = gtk::StackSidebar::new();
        sidebar.set_stack(&stack);
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        root.set_vexpand(true);
        root.append(&sidebar);
        root.append(&stack);
        let content = dialog.content_area();
        content.set_spacing(12);
        content.set_margin_start(12);
        content.set_margin_end(12);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.append(&root);
        let error_label = gtk::Label::new(None);
        error_label.set_xalign(0.0);
        error_label.set_wrap(true);
        content.append(&error_label);

        let binding = Rc::new_cyclic(|weak: &Weak<Self>| {
            let observer = weak.clone();
            // Revision covers projections, validation and capture, but fires
            // inside update's controller borrow. Always defer observer work.
            let subscription = controller.vm.revision.observe(move |_| {
                if let Some(binding) = observer.upgrade() {
                    binding.schedule();
                }
            });
            Self {
                self_weak: weak.clone(),
                controller: RefCell::new(controller),
                _subscription: subscription,
                dialog: dialog.downgrade(),
                stack: stack.clone(),
                error_label,
                registry: RefCell::default(),
                refreshing: Cell::new(false),
                updating: Cell::new(false),
                pending: Cell::new(false),
                scheduled: Cell::new(false),
                draining: Cell::new(false),
                events: RefCell::default(),
                chooser: RefCell::default(),
                closed: Cell::new(false),
            }
        });
        stack.connect_visible_child_notify({
            let weak = Rc::downgrade(&binding);
            move |stack| {
                let Some(binding) = weak.upgrade() else {
                    return;
                };
                if binding.refreshing.get() {
                    return;
                }
                let id = stack
                    .visible_child()
                    .and_then(|child| binding.registry.borrow().page_control(&child));
                if let Some(id) = id {
                    binding.input(&id, ControlInput::Activate);
                }
            }
        });
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed({
            let weak = Rc::downgrade(&binding);
            move |_, key, _, _| {
                let Some(binding) = weak.upgrade() else {
                    return glib::Propagation::Proceed;
                };
                let screen = binding.controller.borrow().screen();
                if screen.capture.target.is_none() {
                    return glib::Propagation::Proceed;
                }
                if let Ok(key) = nerust_keyboard::Key::try_from(key)
                    && let Some(events) = screen.bind_capture_key(key)
                {
                    binding.send(events);
                }
                glib::Propagation::Stop
            }
        });
        dialog.add_controller(keys);
        binding.refresh();
        binding
    }

    fn schedule(&self) {
        if self.closed.get() || self.scheduled.replace(true) {
            return;
        }
        let weak = self.self_weak.clone();
        glib::idle_add_local_once(move || {
            if let Some(binding) = weak.upgrade() {
                binding.scheduled.set(false);
                binding.drain();
            }
        });
    }

    fn input(&self, id: &SettingsControlId, input: ControlInput) {
        if self.closed.get() || self.refreshing.get() {
            return;
        }
        // Fresh resolution supplies current choices, enablement and context.
        let events = self.controller.borrow().screen().bind(id, input);
        if let Some(events) = events {
            self.send(events);
        }
    }

    fn send(&self, events: impl IntoIterator<Item = SettingsEvent>) {
        if self.closed.get() {
            return;
        }
        self.events.borrow_mut().extend(events);
        self.drain();
    }

    fn drain(&self) {
        if self.closed.get() || self.draining.get() {
            return;
        }
        let mut effects = Vec::new();
        {
            let _guard = FlagGuard::enter(&self.draining);
            loop {
                let Ok(mut controller) = self.controller.try_borrow_mut() else {
                    self.schedule();
                    return;
                };
                let event = self.events.borrow_mut().pop_front();
                let Some(event) = event else { break };
                if let Some(effect) = controller.update(event) {
                    effects.push(effect);
                }
            }
        }
        // Navigation, invalid edits and controller errors can leave revision unchanged.
        self.refresh();
        for effect in effects {
            self.effect(effect);
        }
    }

    fn refresh(&self) {
        if self.closed.get() {
            return;
        }
        if self.updating.get() {
            self.pending.set(true);
            return;
        }
        let _updating = FlagGuard::enter(&self.updating);
        loop {
            self.pending.set(false);
            let window = {
                let Ok(controller) = self.controller.try_borrow() else {
                    self.schedule();
                    return;
                };
                // This controller is the only settings draft. The auxiliary
                // registry's PendingSettings is just a lifetime reservation.
                controller.window()
            };
            // No controller/registry borrow across any synchronous native setter.
            let _signals = FlagGuard::enter(&self.refreshing);
            window.visit(&mut crate::window::GtkWindowRenderer::Settings(self));
            if self.closed.get() || !self.pending.get() {
                break;
            }
        }
    }

    fn effect(&self, effect: SettingsEffect) {
        let SettingsEffect::BrowseStorageDirectory { title } = effect;
        if self.closed.get() {
            return;
        }
        let existing = self.chooser.borrow().clone();
        if let Some(chooser) = existing {
            chooser.show();
            return;
        }
        let Some(dialog) = self.dialog.upgrade() else {
            return;
        };
        let chooser = gtk::FileChooserNative::new(
            Some(&title),
            Some(&dialog),
            gtk::FileChooserAction::SelectFolder,
            None,
            None,
        );
        chooser.connect_response({
            let weak = self.self_weak.clone();
            move |chooser, response| {
                let path = (response == gtk::ResponseType::Accept)
                    .then(|| chooser.file().and_then(|file| file.path()))
                    .flatten();
                chooser.hide();
                if let Some(binding) = weak.upgrade() {
                    let old = {
                        let mut current = binding.chooser.borrow_mut();
                        if current.as_ref() != Some(chooser) {
                            return;
                        }
                        current.take()
                    };
                    drop(old);
                    binding.send([SettingsEvent::StorageDirectorySelected(path)]);
                }
                chooser.destroy();
            }
        });
        *self.chooser.borrow_mut() = Some(chooser.clone());
        chooser.show();
    }

    fn close(&self) {
        self.closed.set(true);
        self.events.borrow_mut().clear();
        let chooser = self.chooser.borrow_mut().take();
        if let Some(chooser) = chooser {
            chooser.destroy();
        }
    }
}

/// Native controls and response effects only. The common window path supplies
/// the single host controller; this adapter never fetches or validates a draft.
pub(crate) fn build(
    model: &HostWindow,
    parent: &gtk::ApplicationWindow,
    state: Rc<RefCell<State>>,
    controller: SettingsController,
    on_close: impl FnOnce() + 'static,
) -> gtk::Dialog {
    let dialog = gtk::Dialog::builder()
        .transient_for(parent)
        .modal(true)
        .destroy_with_parent(true)
        .default_width(900)
        .default_height(560)
        .build();
    let binding = SettingsBinding::new(controller, &dialog);
    {
        let _signals = FlagGuard::enter(&binding.refreshing);
        model.visit(&mut crate::window::GtkWindowRenderer::Settings(&binding));
    }
    let finish: FinishCallback = Rc::new(RefCell::new(Some(Box::new({
        let weak = Rc::downgrade(&binding);
        move || {
            if let Some(binding) = weak.upgrade() {
                binding.close();
            }
            on_close();
        }
    }))));
    crate::native::dialog::connect_finish(&dialog, &finish);
    let parent = parent.clone();
    let responding = Cell::new(false);
    dialog.connect_response(move |dialog, response| {
        if binding.closed.get() || responding.get() {
            return;
        }
        let _guard = FlagGuard::enter(&responding);
        if response != gtk::ResponseType::Ok {
            binding.input(&SettingsControlId::Cancel, ControlInput::Activate);
            binding.close();
            dialog.destroy();
            crate::native::dialog::finish(&finish);
            return;
        }
        binding.input(&SettingsControlId::Submit, ControlInput::Activate);
        let snapshot = binding.controller.borrow().take_pending_apply();
        let Some(snapshot) = snapshot else { return };
        let scaling = snapshot.local.video.window.scaling;
        let fullscreen = snapshot.local.video.window.fullscreen_default;
        let result = state.borrow_mut().apply_settings(snapshot);
        match result {
            Ok(result) => {
                if result.fullscreen_default_changed {
                    parent.set_fullscreened(fullscreen);
                }
                if result.scaling_changed {
                    let profile = state.borrow().render_profile();
                    if let Some(profile) = profile {
                        apply_scaling_to_window(&parent, scaling, &profile);
                    }
                }
                binding.close();
                dialog.destroy();
                crate::native::dialog::finish(&finish);
            }
            Err(error) => {
                binding.controller.borrow_mut().error_message = Some(error.to_string());
                binding.refresh();
            }
        }
    });
    dialog
}

use crate::native::dialog::FinishCallback;

fn apply_scaling_to_window(
    window: &gtk::ApplicationWindow,
    scaling: ScalingMode,
    profile: &nerust_render_traits::VideoRenderProfile,
) {
    let scale = match scaling {
        ScalingMode::FitToWindow => None,
        ScalingMode::X1 => Some(1),
        ScalingMode::X2 => Some(2),
        ScalingMode::X3 => Some(3),
        ScalingMode::X4 => Some(4),
        ScalingMode::X5 => Some(5),
    };
    if let Some(scale) = scale {
        window.set_default_size(
            profile.physical_size.width as i32 * scale,
            profile.physical_size.height as i32 * scale,
        );
    }
    window.queue_resize();
}

#[cfg(test)]
#[path = "settings/tests.rs"]
mod tests;

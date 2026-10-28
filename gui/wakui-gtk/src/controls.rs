//! Domain-free retained controls and native lifetime. The app owns the draft;
//! the binding holds only a screen snapshot, an event queue, and a mailbox.
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    path::PathBuf,
    rc::{Rc, Weak},
};

use gtk::{glib, prelude::*};
use wakui::control::{ControlEffect, ControlEvent, ControlId, ControlInput, ControlScreen};
use wakui::mailbox::WindowMailbox;
use wakui::window::Window as HostWindow;

#[path = "renderer.rs"]
pub mod renderer;
use renderer::{ControlGtkVisitor, Registry};

/// Each callback completes its owner borrow before returning to native code.
pub struct Callbacks {
    pub current: Box<dyn Fn() -> bool>,
    /// Pull the latest app screen; history refresh happens app-side first.
    pub refresh: Box<dyn Fn() -> Option<ControlScreen>>,
    /// Dispatch queued events through the app; picker effects run natively.
    pub submit: Box<dyn Fn(Vec<ControlEvent>) -> Vec<ControlEffect>>,
    /// Complete a native picker round-trip through the app.
    pub submit_picker: Box<dyn Fn(ControlId, wakui::control::Id, Option<PathBuf>)>,
    pub close: Box<dyn Fn()>,
}

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

pub struct WidgetBinding {
    self_weak: Weak<Self>,
    screen: RefCell<ControlScreen>,
    mailbox: WindowMailbox,
    callbacks: Callbacks,
    dialog: glib::WeakRef<gtk::Dialog>,
    stack: gtk::Stack,
    sidebar: gtk::StackSidebar,
    error_label: gtk::Label,
    registry: RefCell<Registry>,
    refreshing: Cell<bool>,
    updating: Cell<bool>,
    pending: Cell<bool>,
    scheduled: Cell<bool>,
    draining: Cell<bool>,
    events: RefCell<VecDeque<ControlEvent>>,
    chooser: RefCell<Option<gtk::FileChooserNative>>,
    follow: RefCell<Option<glib::SourceId>>,
    parent_finish: RefCell<Option<(glib::WeakRef<gtk::ApplicationWindow>, glib::SignalHandlerId)>>,
    closed: Cell<bool>,
}

impl WidgetBinding {
    pub fn dialog(&self) -> Option<gtk::Dialog> {
        self.dialog.upgrade()
    }

    pub fn render(&self) {
        let screen = self.screen.borrow();
        screen.visit(&mut ControlGtkVisitor::new(self));
    }

    /// Owned snapshot of the currently retained screen.
    pub fn screen_snapshot(&self) -> ControlScreen {
        self.screen.borrow().clone()
    }

    /// Retained widget registry for host-driven pumps and assertions.
    pub fn registry(&self) -> std::cell::Ref<'_, renderer::Registry> {
        self.registry.borrow()
    }

    /// Page stack for host-driven page selection.
    pub fn stack(&self) -> gtk::Stack {
        self.stack.clone()
    }

    fn new(
        screen: ControlScreen,
        mailbox: WindowMailbox,
        dialog: &gtk::Dialog,
        callbacks: Callbacks,
    ) -> Rc<Self> {
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
        let binding = Rc::new_cyclic(|weak| Self {
            self_weak: weak.clone(),
            screen: RefCell::new(screen),
            mailbox,
            callbacks,
            dialog: dialog.downgrade(),
            stack: stack.clone(),
            sidebar: sidebar.clone(),
            error_label,
            registry: RefCell::default(),
            refreshing: Cell::new(false),
            updating: Cell::new(false),
            pending: Cell::new(false),
            scheduled: Cell::new(false),
            draining: Cell::new(false),
            events: RefCell::default(),
            chooser: RefCell::default(),
            follow: RefCell::default(),
            parent_finish: RefCell::default(),
            closed: Cell::new(false),
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
                    binding.input(id, ControlInput::Activate);
                }
            }
        });
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed({
            let weak = Rc::downgrade(&binding);
            move |_, key, _, _| {
                let Some(binding) = weak.upgrade() else {
                    return glib::Propagation::Stop;
                };
                let capturing = binding.screen.borrow().capture.target.is_some();
                if !capturing {
                    return glib::Propagation::Proceed;
                }
                if let Some(key) = crate::keyboard::key_from_gdk(key) {
                    binding.send(ControlEvent::capture_key(key));
                }
                glib::Propagation::Stop
            }
        });
        dialog.add_controller(keys);
        binding
    }

    fn live(&self) -> bool {
        !self.closed.get() && (self.callbacks.current)()
    }

    fn schedule(&self) {
        if !self.live() || self.scheduled.replace(true) {
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

    /// Queue one control input for the host drain. The native host calls
    /// this from retained widget signals; effects run on [`Self::drain`].
    pub fn input(&self, id: ControlId, input: ControlInput) {
        if !self.live() || self.refreshing.get() {
            return;
        }
        self.send(ControlEvent::new(id, input));
    }

    fn send(&self, event: ControlEvent) {
        if !self.live() {
            return;
        }
        self.events.borrow_mut().push_back(event);
        if self.updating.get() {
            self.pending.set(true);
            return;
        }
        self.drain();
    }

    fn drain(&self) {
        if !self.live() || self.draining.get() {
            return;
        }
        if self.updating.get() {
            self.pending.set(true);
            return;
        }
        let _guard = FlagGuard::enter(&self.draining);
        let events: Vec<ControlEvent> = std::mem::take(&mut *self.events.borrow_mut())
            .into_iter()
            .collect();
        if events.is_empty() {
            return;
        }
        let effects = (self.callbacks.submit)(events);
        for effect in effects {
            self.effect(effect);
        }
        self.refresh();
        if !self.live() {
            return;
        }
        if self.mailbox.close_requested() {
            self.finish();
        }
    }

    /// Rebuild when the mailbox flagged invalidation. Native follow hook.
    pub fn sync(&self) {
        if !self.live() {
            return;
        }
        if self.updating.get() {
            self.pending.set(true);
            return;
        }
        let dirty = self.mailbox.take_invalidated();
        if dirty {
            self.refresh();
        }
    }

    /// Pull the latest app screen and rebuild retained widgets. The native
    /// host calls this after dispatching events or on the follow timer.
    pub fn refresh(&self) {
        if !self.live() {
            return;
        }
        if self.updating.get() {
            self.pending.set(true);
            return;
        }
        let _updating = FlagGuard::enter(&self.updating);
        loop {
            self.pending.set(false);
            let Some(screen) = (self.callbacks.refresh)() else {
                self.schedule();
                return;
            };
            *self.screen.borrow_mut() = screen;
            let _signals = FlagGuard::enter(&self.refreshing);
            self.render();
            if !self.live() || self.pending.get() || !self.events.borrow().is_empty() {
                break;
            }
        }
        drop(_updating);
        if !self.draining.get() && !self.events.borrow().is_empty() {
            self.drain();
        }
        if self.mailbox.close_requested() {
            self.finish();
        }
    }

    fn effect(&self, effect: ControlEffect) {
        let ControlEffect::PickDirectory {
            title,
            target,
            request,
        } = effect;
        if !self.live() {
            return;
        }
        let old = self.chooser.borrow_mut().take();
        if let Some(old) = old {
            old.destroy();
        }
        let Some(dialog) = self.dialog() else { return };
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
                if let Some(binding) = weak.upgrade() {
                    let old = {
                        let mut current = binding.chooser.borrow_mut();
                        if current.as_ref() != Some(chooser) {
                            return;
                        }
                        current.take()
                    };
                    drop(old);
                    chooser.hide();
                    if binding.live() {
                        (binding.callbacks.submit_picker)(target, request, path);
                        // Mirror drain's post-dispatch repaint: picker results
                        // produce no submission or effects, only a fresh screen.
                        binding.refresh();
                    }
                }
                chooser.destroy();
            }
        });
        *self.chooser.borrow_mut() = Some(chooser.clone());
        chooser.show();
    }

    fn close(&self) -> bool {
        if self.closed.replace(true) {
            return false;
        }
        self.events.borrow_mut().clear();
        let follow = self.follow.borrow_mut().take();
        if let Some(follow) = follow {
            follow.remove();
        }
        let chooser = self.chooser.borrow_mut().take();
        if let Some(chooser) = chooser {
            chooser.destroy();
        }
        let parent_finish = self.parent_finish.borrow_mut().take();
        if let Some((parent, signal)) = parent_finish
            && let Some(parent) = parent.upgrade()
        {
            parent.disconnect(signal);
        }
        true
    }

    fn finish(&self) {
        if !self.close() {
            return;
        }
        (self.callbacks.close)();
        if let Some(dialog) = self.dialog() {
            dialog.destroy();
        }
    }
}

impl Drop for WidgetBinding {
    fn drop(&mut self) {
        self.close();
    }
}

#[derive(Clone)]
pub struct WidgetDialog {
    pub dialog: gtk::Dialog,
    pub binding: Rc<WidgetBinding>,
}

pub fn build(
    _model: &HostWindow,
    parent: &gtk::ApplicationWindow,
    screen: ControlScreen,
    mailbox: WindowMailbox,
    callbacks: Callbacks,
) -> WidgetDialog {
    let dialog = gtk::Dialog::builder()
        .transient_for(parent)
        .destroy_with_parent(true)
        .default_width(900)
        .default_height(560)
        .build();
    let binding = WidgetBinding::new(screen, mailbox, &dialog, callbacks);
    // GTK's destroy-with-parent does not reliably notify a retained dialog.
    // Finish the program explicitly when its parent loses the native surface.
    let weak = Rc::downgrade(&binding);
    let signal = parent.connect_unrealize(move |_| {
        if let Some(binding) = weak.upgrade() {
            binding.finish();
        }
    });
    *binding.parent_finish.borrow_mut() = Some((parent.downgrade(), signal));
    {
        let _signals = FlagGuard::enter(&binding.refreshing);
        binding.render();
    }
    // The dialog keeps its program alive; the program only weakly references it.
    // Action-area buttons (footer Cancel/OK) emit ResponseType::None;
    // their input events drive submit/close, so the response is ignored.
    dialog.connect_response({
        let binding = binding.clone();
        move |_, response| {
            if response != gtk::ResponseType::None {
                binding.finish()
            }
        }
    });
    let finish: crate::dialog::FinishCallback = Rc::new(RefCell::new(Some(Box::new({
        let weak = Rc::downgrade(&binding);
        move || {
            if let Some(binding) = weak.upgrade() {
                binding.finish();
            }
        }
    }))));
    crate::dialog::connect_finish(&dialog, &finish);
    let weak = Rc::downgrade(&binding);
    let source = glib::timeout_add_local(std::time::Duration::from_millis(500), move || {
        let Some(binding) = weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        if !binding.live() {
            // The currently executing source must not be removed again.
            binding.follow.borrow_mut().take();
            return glib::ControlFlow::Break;
        }
        binding.sync();
        glib::ControlFlow::Continue
    });
    *binding.follow.borrow_mut() = Some(source);
    WidgetDialog { dialog, binding }
}

#[cfg(test)]
mod self_tests {
    use super::FlagGuard;
    use std::cell::Cell;

    #[test]
    fn refreshing_guard_restores_previous_nested_state() {
        let flag = Cell::new(false);
        {
            let _outer = FlagGuard::enter(&flag);
            {
                let _inner = FlagGuard::enter(&flag);
                assert!(flag.get());
            }
            assert!(flag.get());
        }
        assert!(!flag.get());
    }
}

//! GTK4 backend. Construct and use views on the GTK initialization thread.
//! `GtkHost` attaches to an existing GTK main loop; `run` owns a native loop.
//! Virtual rows have a fixed, clipped height in logical pixels (no overscan).
//! Item keys must be unique; invalid snapshots leave the last valid UI intact.
mod controls;
pub use gtk;

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    panic::{AssertUnwindSafe, catch_unwind},
    rc::{Rc, Weak},
    time::Duration,
};

use futures::future::{AbortHandle, Abortable, LocalBoxFuture};
use gtk::{gio, glib, prelude::*};
use panechord_runtime::{
    Binding, Command, Executor, Keyed, Layout, MenuItem, MenuRole, Session, Shortcut,
    ShutdownPhase, Store, Style, Subscription, UiHost, Viewport, WindowHooks,
};

/// A native container, not a second runtime parent descriptor.
#[derive(Clone)]
pub struct Parent {
    widget: gtk::Widget,
    kind: ParentKind,
    next: Rc<Cell<u32>>,
}

#[derive(Clone, Copy)]
enum ParentKind {
    Stack,
    Grid(u32),
    Flex,
    Absolute,
    Notebook,
}

impl Parent {
    fn new(widget: &impl IsA<gtk::Widget>, kind: ParentKind) -> Self {
        Self {
            widget: widget.as_ref().clone(),
            kind,
            next: Rc::new(Cell::new(0)),
        }
    }

    fn append(&self, widget: &gtk::Widget) {
        let index = self.next.get();
        self.next.set(index + 1);
        match self.kind {
            ParentKind::Stack => self
                .widget
                .downcast_ref::<gtk::Box>()
                .unwrap()
                .append(widget),
            ParentKind::Grid(columns) => self.widget.downcast_ref::<gtk::Grid>().unwrap().attach(
                widget,
                (index % columns) as i32,
                (index / columns) as i32,
                1,
                1,
            ),
            ParentKind::Flex => self
                .widget
                .downcast_ref::<gtk::FlowBox>()
                .unwrap()
                .insert(widget, -1),
            ParentKind::Absolute => self
                .widget
                .downcast_ref::<gtk::Fixed>()
                .unwrap()
                .put(widget, 0.0, 0.0),
            ParentKind::Notebook => {
                self.widget
                    .downcast_ref::<gtk::Notebook>()
                    .unwrap()
                    .append_page(widget, None::<&gtk::Widget>);
            }
        }
    }

    fn remove(&self, widget: &gtk::Widget) {
        match self.kind {
            ParentKind::Stack => self
                .widget
                .downcast_ref::<gtk::Box>()
                .unwrap()
                .remove(widget),
            ParentKind::Grid(_) => self
                .widget
                .downcast_ref::<gtk::Grid>()
                .unwrap()
                .remove(widget),
            ParentKind::Flex => {
                if let Some(child) = widget.parent().and_downcast::<gtk::FlowBoxChild>() {
                    self.widget
                        .downcast_ref::<gtk::FlowBox>()
                        .unwrap()
                        .remove(&child);
                }
            }
            ParentKind::Absolute => self
                .widget
                .downcast_ref::<gtk::Fixed>()
                .unwrap()
                .remove(widget),
            ParentKind::Notebook => {
                let notebook = self.widget.downcast_ref::<gtk::Notebook>().unwrap();
                if let Some(page) = notebook.page_num(widget) {
                    notebook.remove_page(Some(page));
                }
            }
        }
    }
}

struct Signal(glib::WeakRef<glib::Object>, Option<glib::SignalHandlerId>);
impl Drop for Signal {
    fn drop(&mut self) {
        if let (Some(object), Some(id)) = (self.0.upgrade(), self.1.take()) {
            object.disconnect(id);
        }
    }
}

#[derive(Default)]
struct Resources {
    tokens: Vec<Subscription>,
    signals: Vec<Signal>,
}

struct Node {
    widget: gtk::Widget,
    frame: gtk::Box,
    parent: Parent,
    css: gtk::CssProvider,
}

trait ListControl {
    fn viewport(&self, start: usize);
    fn keys(&self) -> Vec<String>;
    fn error(&self) -> Option<String>;
}

struct MenuState {
    menu: gio::Menu,
    title: RefCell<String>,
    actions: RefCell<Vec<String>>,
    shortcuts: RefCell<Vec<gtk::Shortcut>>,
}

struct Action {
    action: gio::SimpleAction,
    name: String,
}

struct Inner {
    window: gtk::Window,
    root: Parent,
    menus: gio::Menu,
    menu_states: RefCell<HashMap<String, Rc<MenuState>>>,
    group: gio::SimpleActionGroup,
    shortcuts: gtk::ShortcutController,
    serial: Cell<u64>,
    scope: RefCell<String>,
    nodes: RefCell<HashMap<String, Node>>,
    resources: RefCell<HashMap<String, Resources>>,
    lists: RefCell<HashMap<String, Rc<dyn ListControl>>>,
    actions: RefCell<HashMap<String, Action>>,
    control_actions: RefCell<HashMap<String, controls::Action>>,
    tabs: RefCell<HashMap<String, Rc<controls::Tab>>>,
    on_close: RefCell<Vec<Rc<dyn Fn()>>>,
    on_close_request: RefCell<Option<Rc<dyn Fn()>>>,
    requesting_close: Cell<bool>,
    coordinated_close: RefCell<Option<Rc<CoordinatedClose>>>,
    closed: Cell<bool>,
    image_updates: Cell<usize>,
    error: RefCell<Option<String>>,
}

#[derive(Clone)]
pub struct View(Rc<Inner>);

/// A non-owning UI-thread handle. Closed or dropped Views cannot be upgraded.
#[derive(Clone)]
pub struct WeakView(Weak<Inner>);
impl WeakView {
    pub fn upgrade(&self) -> Option<View> {
        self.0
            .upgrade()
            .filter(|inner| !inner.closed.get())
            .map(View)
    }
}

type Template<T> = Rc<dyn Fn(&View, &Parent, &str, Store<T>)>;

fn scoped(id: &str, prefix: &str) -> bool {
    id == prefix
        || id
            .strip_prefix(prefix)
            .is_some_and(|tail| tail.starts_with('/'))
}

impl Inner {
    fn shutdown(&self) {
        if self.closed.replace(true) {
            return;
        }
        drop(self.coordinated_close.take());
        drop(self.on_close_request.take());
        // Never run token, signal or application destructors under a registry borrow.
        drop(self.resources.take());
        drop(self.lists.take());
        drop(self.control_actions.take());
        drop(self.tabs.take());
        self.menus.remove_all();
        self.window.remove_controller(&self.shortcuts);
        for (_, state) in self.menu_states.take() {
            for shortcut in state.shortcuts.take() {
                self.shortcuts.remove_shortcut(&shortcut);
            }
        }
        for (_, action) in self.actions.take() {
            self.group.remove_action(&action.name);
        }
        for callback in self.on_close.take() {
            if let Err(panic) = catch_unwind(AssertUnwindSafe(|| callback())) {
                self.error.replace(Some(panic_error(panic)));
            }
        }
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.shutdown();
        self.window.destroy();
    }
}

impl View {
    pub fn new(title: Binding<String>) -> Self {
        let window = gtk::Window::builder()
            .default_width(660)
            .default_height(520)
            .build();
        let outer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.set_vexpand(true);
        let menus = gio::Menu::new();
        outer.append(&gtk::PopoverMenuBar::from_model(Some(&menus)));
        let canvas = gtk::Box::new(gtk::Orientation::Vertical, 0);
        canvas.append(&root);
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .hexpand(true)
            .vexpand(true)
            .child(&canvas)
            .build();
        outer.append(&scroll);
        window.set_child(Some(&outer));
        root.set_widget_name("__root");
        let css = gtk::CssProvider::new();
        root.style_context()
            .add_provider(&css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
        let group = gio::SimpleActionGroup::new();
        window.insert_action_group("panechord", Some(&group));
        let shortcuts = gtk::ShortcutController::new();
        shortcuts.set_scope(gtk::ShortcutScope::Global);
        window.add_controller(shortcuts.clone());
        let view = Self(Rc::new(Inner {
            window,
            root: Parent::new(&root, ParentKind::Stack),
            menus,
            group,
            shortcuts,
            serial: Cell::new(0),
            scope: RefCell::default(),
            nodes: RefCell::new(HashMap::from([(
                "__root".into(),
                Node {
                    widget: root.clone().upcast(),
                    frame: root,
                    parent: Parent::new(&canvas, ParentKind::Stack),
                    css,
                },
            )])),
            resources: RefCell::default(),
            lists: RefCell::default(),
            actions: RefCell::default(),
            control_actions: RefCell::default(),
            tabs: RefCell::default(),
            menu_states: RefCell::default(),
            on_close: RefCell::default(),
            on_close_request: RefCell::default(),
            requesting_close: Cell::new(false),
            coordinated_close: RefCell::default(),
            closed: Cell::new(false),
            image_updates: Cell::new(0),
            error: RefCell::default(),
        }));
        let weak = Rc::downgrade(&view.0);
        let signal = view.0.window.connect_close_request(move |_| {
            if let Some(inner) = weak.upgrade() {
                View(inner).request_close();
            }
            glib::Propagation::Stop
        });
        view.signal("", &view.0.window, signal);
        view.bind_native("", title, &view.0.window, |window, title| {
            window.set_title(Some(&title))
        });
        view
    }

    pub fn root(&self) -> Parent {
        self.0.root.clone()
    }

    /// Obtain the real window for integration with an existing GTK Application.
    pub fn native_window(&self) -> gtk::Window {
        self.0.window.clone()
    }

    /// Construct custom GTK primitives in application code, then mount them here.
    /// The ID participates in style, automation and template-prefix cleanup.
    pub fn mount_native(&self, parent: &Parent, id: &str, widget: &impl IsA<gtk::Widget>) {
        assert!(!self.0.closed.get(), "cannot mount into a closed view");
        assert!(!id.is_empty(), "empty control ID is reserved");
        assert!(
            !self.0.nodes.borrow().contains_key(id),
            "duplicate view id: {id}"
        );
        assert!(
            widget.parent().is_none(),
            "native widget already has a parent"
        );
        let frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
        frame.set_widget_name(id);
        frame.set_hexpand(true);
        widget.set_widget_name(id);
        frame.append(widget);
        let css = gtk::CssProvider::new();
        frame
            .style_context()
            .add_provider(&css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
        parent.append(frame.upcast_ref());
        self.0.nodes.borrow_mut().insert(
            id.into(),
            Node {
                widget: widget.as_ref().clone(),
                frame,
                parent: parent.clone(),
                css,
            },
        );
    }

    /// Weak native binding, detached with the ID on eviction or View shutdown.
    pub fn bind_native<T: Clone + 'static, O: IsA<glib::Object> + Clone + 'static>(
        &self,
        id: &str,
        binding: Binding<T>,
        object: &O,
        apply: impl Fn(&O, T) + 'static,
    ) {
        let object = object.downgrade();
        let owner = Rc::downgrade(&self.0);
        let token = binding.subscribe(Rc::new(move |value| {
            if owner.upgrade().is_some_and(|inner| !inner.closed.get())
                && let Some(object) = object.upgrade()
            {
                apply(&object, value);
            }
        }));
        self.keep_at(id, token);
    }

    fn keep_at(&self, id: &str, token: Subscription) {
        if !self.0.closed.get() {
            self.0
                .resources
                .borrow_mut()
                .entry(id.into())
                .or_default()
                .tokens
                .push(token);
        }
    }

    pub fn keep(&self, token: Subscription) {
        let scope = self.0.scope.borrow().clone();
        self.keep_at(&scope, token);
    }

    fn in_scope(&self, prefix: &str, build: impl FnOnce()) {
        struct Restore<'a>(&'a RefCell<String>, Option<String>);
        impl Drop for Restore<'_> {
            fn drop(&mut self) {
                self.0.replace(self.1.take().unwrap());
            }
        }
        let previous = self.0.scope.replace(prefix.into());
        let _restore = Restore(&self.0.scope, Some(previous));
        build();
    }

    /// Custom primitives should register native signals here for scoped teardown.
    pub fn track_signal(
        &self,
        id: &str,
        object: &impl IsA<glib::Object>,
        signal: glib::SignalHandlerId,
    ) {
        self.signal(id, object, signal);
    }

    fn signal(&self, id: &str, object: &impl IsA<glib::Object>, signal: glib::SignalHandlerId) {
        let signal = Signal(object.as_ref().downgrade(), Some(signal));
        if !self.0.closed.get() {
            self.0
                .resources
                .borrow_mut()
                .entry(id.into())
                .or_default()
                .signals
                .push(signal);
        }
    }

    fn cleanup(&self, prefix: &str) {
        self.0
            .control_actions
            .borrow_mut()
            .retain(|id, _| !scoped(id, prefix));
        self.0.tabs.borrow_mut().retain(|id, _| !scoped(id, prefix));
        let resource_ids: Vec<_> = self
            .0
            .resources
            .borrow()
            .keys()
            .filter(|id| scoped(id, prefix))
            .cloned()
            .collect();
        let resources: Vec<_> = {
            let mut registry = self.0.resources.borrow_mut();
            resource_ids
                .iter()
                .filter_map(|id| registry.remove(id))
                .collect()
        };
        drop(resources);
        let list_ids: Vec<_> = self
            .0
            .lists
            .borrow()
            .keys()
            .filter(|id| scoped(id, prefix))
            .cloned()
            .collect();
        let lists: Vec<_> = {
            let mut registry = self.0.lists.borrow_mut();
            list_ids
                .iter()
                .filter_map(|id| registry.remove(id))
                .collect()
        };
        drop(lists);
        let menu_ids: Vec<_> = self
            .0
            .menu_states
            .borrow()
            .keys()
            .filter(|id| scoped(id, prefix))
            .cloned()
            .collect();
        for id in menu_ids {
            self.clear_menu(&id);
            self.0.menu_states.borrow_mut().remove(&id);
        }
        self.rebuild_menubar();
        let mut ids: Vec<_> = self
            .0
            .nodes
            .borrow()
            .keys()
            .filter(|id| scoped(id, prefix))
            .cloned()
            .collect();
        ids.sort_by_key(|id| std::cmp::Reverse(id.len()));
        for id in ids {
            let node = self.0.nodes.borrow_mut().remove(&id);
            if let Some(node) = node {
                node.parent.remove(node.frame.upcast_ref());
            }
        }
    }

    pub fn layout(&self, parent: &Parent, id: &str, layout: Layout) -> Parent {
        self.try_layout(parent, id, layout).expect("invalid layout")
    }
    pub fn try_layout(&self, parent: &Parent, id: &str, layout: Layout) -> Result<Parent, String> {
        layout.validate().map_err(str::to_owned)?;
        if self.0.closed.get() || id.is_empty() || self.0.nodes.borrow().contains_key(id) {
            return Err("closed view, empty or duplicate layout id".into());
        }
        if !parent
            .widget
            .root()
            .and_then(|root| root.downcast::<gtk::Window>().ok())
            .is_some_and(|window| {
                window == self.0.window || window.transient_for().as_ref() == Some(&self.0.window)
            })
        {
            return Err("foreign or unavailable layout parent".into());
        }
        let result = match layout {
            Layout::Stack { horizontal, gap } => {
                let widget = gtk::Box::new(
                    if horizontal {
                        gtk::Orientation::Horizontal
                    } else {
                        gtk::Orientation::Vertical
                    },
                    pixels(gap),
                );
                Parent::new(&widget, ParentKind::Stack)
            }
            Layout::Grid { columns, gap } => {
                assert!(columns > 0, "Grid columns must be positive");
                let widget = gtk::Grid::new();
                widget.set_column_homogeneous(true);
                widget.set_hexpand(true);
                // GTK only counts attached tracks; retain even empty declared
                // columns, otherwise a partially populated grid changes ratios.
                for column in 0..columns {
                    let track = gtk::Box::new(gtk::Orientation::Vertical, 0);
                    track.set_hexpand(true);
                    widget.attach(&track, column as i32, 0, 1, 1);
                }
                widget.set_row_spacing(pixels(gap) as u32);
                widget.set_column_spacing(pixels(gap) as u32);
                Parent::new(&widget, ParentKind::Grid(columns))
            }
            Layout::Flex { gap } => {
                let widget = gtk::FlowBox::new();
                widget.set_selection_mode(gtk::SelectionMode::None);
                widget.set_row_spacing(pixels(gap) as u32);
                widget.set_column_spacing(pixels(gap) as u32);
                widget.set_min_children_per_line(1);
                widget.set_max_children_per_line(u16::MAX as u32);
                Parent::new(&widget, ParentKind::Flex)
            }
            Layout::Absolute => Parent::new(&gtk::Fixed::new(), ParentKind::Absolute),
        };
        self.mount_native(parent, id, &result.widget);
        Ok(result)
    }

    pub fn style(&self, id: &str, style: Style) {
        if let Err(error) = self.try_style(id, style) {
            self.0.error.replace(Some(error));
        }
    }
    pub fn try_style(&self, id: &str, style: Style) -> Result<(), String> {
        if self.0.closed.get() {
            return Err("view is closed".into());
        }
        let (frame, parent, css) = {
            let nodes = self.0.nodes.borrow();
            let node = nodes
                .get(id)
                .ok_or_else(|| format!("style target not mounted: {id}"))?;
            (node.frame.clone(), node.parent.clone(), node.css.clone())
        };
        let layout = match parent.kind {
            ParentKind::Grid(columns) => Layout::Grid { columns, gap: 0.0 },
            ParentKind::Absolute => Layout::Absolute,
            _ => Layout::Stack {
                horizontal: false,
                gap: 0.0,
            },
        };
        style.validate(layout).map_err(str::to_owned)?;
        let margin = style.margin;
        frame.set_margin_top(pixels(margin.top));
        frame.set_margin_end(pixels(margin.right));
        frame.set_margin_bottom(pixels(margin.bottom));
        frame.set_margin_start(pixels(margin.left));
        let padding = style.padding;
        css.load_from_data(&format!(
            "* {{ padding: {}px {}px {}px {}px; }}",
            pixels(padding.top),
            pixels(padding.right),
            pixels(padding.bottom),
            pixels(padding.left)
        ));
        let p = style.placement;
        frame.set_size_request(
            p.width.map(pixels).unwrap_or(-1),
            p.height.map(pixels).unwrap_or(-1),
        );
        match parent.kind {
            ParentKind::Grid(_) => {
                let grid = parent.widget.downcast_ref::<gtk::Grid>().unwrap();
                grid.remove(&frame);
                grid.attach(&frame, p.column, p.row, p.column_span, p.row_span);
            }
            ParentKind::Absolute => {
                parent
                    .widget
                    .downcast_ref::<gtk::Fixed>()
                    .unwrap()
                    .move_(&frame, p.x as f64, p.y as f64);
            }
            _ => {}
        }
        Ok(())
    }

    pub fn label(&self, parent: &Parent, id: &str, text: Binding<String>) {
        let label = gtk::Label::new(None);
        label.set_xalign(0.0);
        label.set_wrap(true);
        label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        label.set_wrap(true);
        self.mount_native(parent, id, &label);
        self.bind_native(id, text, &label, |label, text| label.set_text(&text));
    }

    pub fn entry(&self, parent: &Parent, id: &str, text: Binding<String>) {
        let entry = gtk::Entry::new();
        entry.set_editable(text.is_writable());
        self.mount_native(parent, id, &entry);
        let applying = Rc::new(Cell::new(false));
        let guard = applying.clone();
        let input = text.clone();
        // set_text emits changed for both deletion and insertion. GTK freezes
        // text notifications across that replacement, so rollback here cannot
        // interrupt its insertion. Native insert/delete edits still notify now.
        let signal = entry.connect_text_notify(move |entry| {
            if !guard.get() {
                let value = entry.text().to_string();
                // Normalization inside notify can queue another notification
                // until after the applying guard is reset. Do not echo it.
                if value == input.get() {
                    return;
                }
                let _ = input.set(value);
                // A failed conversion/read-only writer must not leave a false display value.
                let actual = input.get();
                if entry.text().as_str() != actual {
                    guard.set(true);
                    entry.set_text(&actual);
                    guard.set(false);
                }
            }
        });
        self.signal(id, &entry, signal);
        self.bind_native(id, text, &entry, move |entry, text| {
            let previous = applying.replace(true);
            if entry.text().as_str() != text {
                entry.set_text(&text);
            }
            applying.set(previous);
        });
    }

    pub fn button(&self, parent: &Parent, id: &str, text: Binding<String>, command: Command) {
        let button = gtk::Button::new();
        self.mount_native(parent, id, &button);
        self.bind_native(id, text, &button, |button, text| button.set_label(&text));
        self.bind_native(id, command.enabled.clone(), &button, |button, enabled| {
            button.set_sensitive(enabled)
        });
        let signal = button.connect_clicked(move |_| {
            let _ = command.invoke();
        });
        self.signal(id, &button, signal);
    }

    pub fn meter(&self, parent: &Parent, id: &str, value: Binding<f64>) {
        let meter = gtk::ProgressBar::new();
        self.mount_native(parent, id, &meter);
        self.bind_native(id, value, &meter, |meter, value| {
            meter.set_fraction(if value.is_finite() {
                value.clamp(0.0, 1.0)
            } else {
                0.0
            });
        });
    }

    // Keep the callback signature explicit in the public backend API.
    #[allow(clippy::type_complexity)]
    pub fn items<T: Clone + 'static>(
        &self,
        parent: &Parent,
        id: &str,
        items: Binding<Vec<Keyed<T>>>,
        viewport: Binding<Viewport>,
        template: Rc<dyn Fn(&View, &Parent, &str, Store<T>)>,
    ) {
        let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let top = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let bottom = gtk::Box::new(gtk::Orientation::Vertical, 0);
        container.append(&top);
        container.append(&bottom);
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .child(&container)
            .build();
        scroll.set_valign(gtk::Align::Start);
        self.mount_native(parent, id, &scroll);
        let rows = Rc::new(Rows {
            owner: Rc::downgrade(&self.0),
            id: id.into(),
            container,
            top,
            bottom,
            scroll,
            source: RefCell::new(Vec::new()),
            mounted: RefCell::new(Vec::new()),
            viewport: Cell::new(Viewport::default()),
            viewport_ready: Cell::new(false),
            binding: viewport.clone(),
            template,
            error: RefCell::new(None),
            viewport_error: RefCell::new(None),
            applying: Cell::new(false),
            queued: Cell::new(false),
        });
        self.0.lists.borrow_mut().insert(id.into(), rows.clone());
        let weak = Rc::downgrade(&rows);
        let adjustment = rows.scroll.vadjustment();
        let signal = adjustment.connect_value_changed(move |adjustment| {
            if let Some(rows) = weak.upgrade() {
                if rows.applying.get() {
                    return;
                }
                let height = pixels(rows.viewport.get().row_height) as f64;
                let start = (adjustment.value() / height).floor().max(0.0) as usize;
                rows.set_start(start, false);
            }
        });
        self.signal(id, &adjustment, signal);
        let weak = Rc::downgrade(&rows);
        self.keep_at(
            id,
            viewport.subscribe(Rc::new(move |viewport| {
                if let Some(rows) = weak.upgrade() {
                    if let Err(error) = viewport.validate() {
                        rows.viewport_error.replace(Some(error.into()));
                        return;
                    }
                    rows.viewport_error.take();
                    rows.viewport_ready.set(true);
                    rows.viewport.set(viewport);
                    rows.refresh(true);
                }
            })),
        );
        let weak = Rc::downgrade(&rows);
        self.keep_at(id, items.subscribe(Rc::new(move |items| {
            if let Some(rows) = weak.upgrade() {
                if let Some(invalid) = items.iter().find(|item| !valid_key(&item.key)) {
                    let error = format!("invalid item key {:?}: keys must be nonempty and contain no '/'; keeping last valid rows", invalid.key);
                    rows.scroll.set_tooltip_text(Some(&error));
                    rows.error.replace(Some(error));
                    return;
                }
                let mut keys = HashSet::new();
                if let Some(duplicate) = items.iter().find(|item| !keys.insert(item.key.clone())) {
                    let error = format!("duplicate item key {:?}; keeping last valid rows", duplicate.key);
                    rows.scroll.set_tooltip_text(Some(&error));
                    rows.error.replace(Some(error));
                    return;
                }
                rows.error.replace(None);
                rows.scroll.set_tooltip_text(None);
                rows.source.replace(items);
                rows.refresh(true);
            }
        })));
    }

    // Keep the callback signature explicit in the public backend API.
    #[allow(clippy::type_complexity)]
    pub fn content<T: Clone + 'static>(
        &self,
        parent: &Parent,
        id: &str,
        item: Binding<Option<Keyed<T>>>,
        template: Rc<dyn Fn(&View, &Parent, &str, Store<T>)>,
    ) {
        let container = self.layout(
            parent,
            id,
            Layout::Stack {
                horizontal: false,
                gap: 0.0,
            },
        );
        let weak = Rc::downgrade(&self.0);
        let prefix = format!("{id}/");
        let current = RefCell::new(None::<Keyed<T>>);
        self.keep_at(id, item.subscribe(Rc::new(move |item| {
            let Some(inner) = weak.upgrade().filter(|inner| !inner.closed.get()) else { return };
            if let Some(invalid) = item.as_ref().filter(|item| !valid_key(&item.key)) {
                let error = format!("invalid content key {:?}: keys must be nonempty and contain no '/'; keeping last valid content", invalid.key);
                container.widget.set_tooltip_text(Some(&error));
                inner.error.replace(Some(error));
                return;
            }
            container.widget.set_tooltip_text(None);
            if *current.borrow() == item { return; }
            let view = View(inner);
            if let Some(old) = current.replace(item.clone()) { view.cleanup(&format!("{prefix}{}", old.key)); }
            if let Some(item) = item {
                let prefix = format!("{prefix}{}", item.key);
                view.in_scope(&prefix, || template(&view, &container, &prefix, item.model));
            }
        })));
    }

    pub fn viewport(&self, id: &str, start: usize) {
        let list = self.0.lists.borrow().get(id).cloned();
        if let Some(list) = list {
            list.viewport(start);
        }
    }
    pub fn item_keys(&self, id: &str) -> Vec<String> {
        self.0
            .lists
            .borrow()
            .get(id)
            .map(|list| list.keys())
            .unwrap_or_default()
    }
    pub fn mounted_count(&self, id: &str) -> usize {
        self.item_keys(id).len()
    }
    pub fn items_error(&self, id: &str) -> Option<String> {
        self.0.lists.borrow().get(id).and_then(|list| list.error())
    }

    pub fn menu(
        &self,
        id: &str,
        title: Binding<String>,
        items: Binding<Vec<MenuItem>>,
        dispatch: Rc<dyn Fn(String) -> bool>,
    ) {
        self.typed_menu(id, title, items, dispatch);
    }
    pub fn typed_menu<A: Clone + 'static>(
        &self,
        id: &str,
        title: Binding<String>,
        items: Binding<Vec<MenuItem<A>>>,
        dispatch: Rc<dyn Fn(A) -> bool>,
    ) {
        assert!(
            !self.0.menu_states.borrow().contains_key(id),
            "duplicate menu id: {id}"
        );
        let state = Rc::new(MenuState {
            menu: gio::Menu::new(),
            title: RefCell::new(String::new()),
            actions: RefCell::new(Vec::new()),
            shortcuts: RefCell::new(Vec::new()),
        });
        self.0
            .menu_states
            .borrow_mut()
            .insert(id.into(), state.clone());
        let weak = Rc::downgrade(&self.0);
        let menu_id = id.to_owned();
        self.keep_at(
            id,
            title.subscribe(Rc::new(move |title| {
                if let Some(inner) = weak.upgrade().filter(|inner| !inner.closed.get()) {
                    if let Some(state) = inner.menu_states.borrow().get(&menu_id) {
                        state.title.replace(title);
                    }
                    View(inner).rebuild_menubar();
                }
            })),
        );
        let weak = Rc::downgrade(&self.0);
        let menu_id = id.to_owned();
        self.keep_at(
            id,
            items.subscribe(Rc::new(move |items| {
                if let Some(inner) = weak.upgrade().filter(|inner| !inner.closed.get()) {
                    View(inner).apply_menu(&menu_id, items, dispatch.clone());
                }
            })),
        );
    }

    fn rebuild_menubar(&self) {
        let mut menus: Vec<_> = self
            .0
            .menu_states
            .borrow()
            .iter()
            .map(|(id, state)| (id.clone(), state.clone()))
            .collect();
        menus.sort_by(|a, b| a.0.cmp(&b.0));
        self.0.menus.remove_all();
        for (_, state) in menus {
            self.0
                .menus
                .append_submenu(Some(&state.title.borrow()), &state.menu);
        }
    }

    fn clear_menu(&self, id: &str) {
        let state = self.0.menu_states.borrow().get(id).cloned();
        let Some(state) = state else { return };
        state.menu.remove_all();
        // Menu signals live in a sub-scope; title/items subscriptions remain alive.
        let signals = self
            .0
            .resources
            .borrow_mut()
            .remove(&format!("{id}/@actions"));
        drop(signals);
        for id in state.actions.take() {
            let action = self.0.actions.borrow_mut().remove(&id);
            if let Some(action) = action {
                self.0.group.remove_action(&action.name);
            }
        }
        for shortcut in state.shortcuts.take() {
            self.0.shortcuts.remove_shortcut(&shortcut);
        }
    }

    fn apply_menu<A: Clone + 'static>(
        &self,
        id: &str,
        items: Vec<MenuItem<A>>,
        dispatch: Rc<dyn Fn(A) -> bool>,
    ) {
        let items: Vec<_> = items
            .into_iter()
            .map(|mut item| {
                item.id = format!("{id}/{}", item.id);
                item
            })
            .collect();
        // Validate the entire snapshot (including shortcuts) before touching the old menu.
        let old_ids: HashSet<_> = self
            .0
            .menu_states
            .borrow()
            .get(id)
            .unwrap()
            .actions
            .borrow()
            .iter()
            .cloned()
            .collect();
        let mut keys = HashSet::new();
        let mut triggers = Vec::new();
        for item in &items {
            if !keys.insert(item.id.clone())
                || (self.0.actions.borrow().contains_key(&item.id) && !old_ids.contains(&item.id))
            {
                self.0
                    .error
                    .replace(Some(format!("duplicate menu item id: {}", item.id)));
                return;
            }
            let trigger = item
                .shortcut
                .as_ref()
                .map(|shortcut| parse_shortcut(shortcut));
            if trigger.as_ref().is_some_and(Option::is_none) {
                self.0
                    .error
                    .replace(Some(format!("invalid GTK shortcut for {}", item.id)));
                return;
            }
            triggers.push(trigger.flatten());
        }
        self.clear_menu(id);
        let state = self.0.menu_states.borrow().get(id).cloned().unwrap();
        let mut section = gio::Menu::new();
        for (item, trigger) in items.into_iter().zip(triggers) {
            if item.role == MenuRole::Separator {
                if section.n_items() > 0 {
                    state.menu.append_section(None, &section);
                }
                section = gio::Menu::new();
                continue;
            }
            let serial = self.0.serial.get();
            self.0.serial.set(serial + 1);
            let name = format!("action{serial}");
            let detailed = format!("panechord.{name}");
            let action = gio::SimpleAction::new(&name, None);
            action.set_enabled(item.enabled);
            let weak = Rc::downgrade(&self.0);
            let dispatch = dispatch.clone();
            let role = item.role.clone();
            let argument = item.argument.clone();
            let signal = action.connect_activate(move |_, _| {
                if let Some(inner) = weak.upgrade().filter(|inner| !inner.closed.get()) {
                    let view = View(inner);
                    if role == MenuRole::Normal {
                        dispatch(argument.clone());
                    } else {
                        view.native_role(&role);
                    }
                }
            });
            self.signal(&format!("{id}/@actions"), &action, signal);
            self.0.group.add_action(&action);
            self.0
                .actions
                .borrow_mut()
                .insert(item.id.clone(), Action { action, name });
            state.actions.borrow_mut().push(item.id);
            section.append(Some(&item.label), Some(&detailed));
            if let Some(trigger) = trigger {
                let shortcut =
                    gtk::Shortcut::new(Some(trigger), Some(gtk::NamedAction::new(&detailed)));
                self.0.shortcuts.add_shortcut(shortcut.clone());
                state.shortcuts.borrow_mut().push(shortcut);
            }
        }
        if section.n_items() > 0 {
            state.menu.append_section(None, &section);
        }
    }

    fn native_role(&self, role: &MenuRole) {
        match role {
            MenuRole::Quit => self.request_close(),
            MenuRole::About => {
                let dialog = gtk::AboutDialog::builder()
                    .transient_for(&self.0.window)
                    .destroy_with_parent(true)
                    .modal(true)
                    .program_name(self.title())
                    .build();
                dialog.present();
            }
            MenuRole::Copy | MenuRole::Cut | MenuRole::Paste | MenuRole::SelectAll => {
                let Some(focus) = GtkWindowExt::focus(&self.0.window) else {
                    return;
                };
                if *role == MenuRole::SelectAll {
                    if let Some(editable) = focus.dynamic_cast_ref::<gtk::Editable>() {
                        editable.select_region(0, -1);
                    } else {
                        let _ = focus.activate_action("selection.select-all", None);
                    }
                } else {
                    let action = match role {
                        MenuRole::Copy => "clipboard.copy",
                        MenuRole::Cut => "clipboard.cut",
                        _ => "clipboard.paste",
                    };
                    let _ = focus.activate_action(action, None);
                }
            }
            MenuRole::Normal | MenuRole::Separator => {}
        }
    }

    pub fn on_close(&self, callback: Rc<dyn Fn()>) {
        if self.0.closed.get() {
            callback();
        } else {
            self.0.on_close.borrow_mut().push(callback);
        }
    }
    pub fn downgrade(&self) -> WeakView {
        WeakView(Rc::downgrade(&self.0))
    }
    /// Replace the single application handler. Capture Store or WeakView, not
    /// this View strongly. A handler decides when to call forceful `close`.
    /// When installed it takes precedence over GtkHost coordinated shutdown.
    pub fn on_close_request(&self, callback: Rc<dyn Fn()>) {
        if !self.0.closed.get() {
            drop(self.0.on_close_request.replace(Some(callback)));
        }
    }
    /// Ordinary user close: app handler, else coordinator, else force teardown.
    /// Synchronous reentrant requests (including native/menu requests) are
    /// ignored, not queued. A later request retries after the callback returns.
    pub fn request_close(&self) {
        if self.0.closed.get() || self.0.requesting_close.replace(true) {
            return;
        }
        let callback = self.0.on_close_request.borrow().clone();
        let coordinator = self.0.coordinated_close.borrow().clone();
        let result = catch_unwind(AssertUnwindSafe(|| {
            if let Some(callback) = callback {
                callback();
            } else if let Some(coordinator) = coordinator {
                coordinator.request();
            } else {
                self.close();
            }
        }));
        self.0.requesting_close.set(false);
        if let Err(panic) = result {
            self.0.error.replace(Some(panic_error(panic)));
        }
    }
    /// Force teardown, bypassing coordinated confirmation/drain. Idempotent and
    /// safe to use from a Session close hook; never emits another close-request.
    pub fn close(&self) {
        self.0.shutdown();
        self.0.window.destroy();
    }
    pub fn present(&self) {
        if !self.0.closed.get() {
            self.0.window.present();
        }
    }
    pub fn title(&self) -> String {
        self.0
            .window
            .title()
            .map(|title| title.to_string())
            .unwrap_or_default()
    }

    fn widget<W: IsA<gtk::Widget>>(&self, id: &str) -> Result<W, String> {
        if self.0.closed.get() {
            return Err("view is closed".into());
        }
        self.0
            .nodes
            .borrow()
            .get(id)
            .map(|node| node.widget.clone())
            .and_then(|widget| widget.downcast::<W>().ok())
            .ok_or_else(|| format!("missing or wrong control: {id}"))
    }
    pub fn edit(&self, id: &str, value: &str) -> Result<(), String> {
        self.require_interactive(id)?;
        let entry = self.widget::<gtk::Entry>(id)?;
        if !entry.is_editable() {
            return Err(format!("read-only entry: {id}"));
        }
        entry.set_text(value);
        Ok(())
    }
    pub fn click(&self, id: &str) -> Result<(), String> {
        self.require_interactive(id)?;
        if let Ok(check) = self.widget::<gtk::CheckButton>(id) {
            if !check.is_sensitive() {
                return Err("read-only control".into());
            }
            let active = if self.0.control_actions.borrow().contains_key(id) {
                !check.is_active()
            } else {
                true
            };
            check.set_active(active);
            return Ok(());
        }
        let button = self.widget::<gtk::Button>(id)?;
        if !button.is_sensitive() {
            return Err(format!("disabled button: {id}"));
        }
        button.emit_clicked();
        Ok(())
    }
    pub fn activate_menu(&self, id: &str) -> Result<(), String> {
        if self.0.closed.get() {
            return Err("view is closed".into());
        }
        let action = {
            let actions = self.0.actions.borrow();
            let action = if id.contains('/') {
                actions.get(id)
            } else {
                let mut matches = actions
                    .iter()
                    .filter(|(key, _)| key.rsplit_once('/').is_some_and(|(_, item)| item == id));
                let first = matches.next().map(|(_, action)| action);
                if matches.next().is_some() {
                    return Err(format!("ambiguous menu item: {id}; use menu/item"));
                }
                first
            };
            action
                .map(|action| action.action.clone())
                .ok_or_else(|| format!("missing menu item: {id}"))?
        };
        if !action.is_enabled() {
            return Err(format!("disabled menu item: {id}"));
        }
        action.activate(None);
        Ok(())
    }
    pub fn text(&self, id: &str) -> Option<String> {
        let widget = self.0.nodes.borrow().get(id)?.widget.clone();
        if let Some(label) = widget.downcast_ref::<gtk::Label>() {
            return Some(label.text().into());
        }
        if let Some(entry) = widget.downcast_ref::<gtk::Entry>() {
            return Some(entry.text().into());
        }
        widget
            .downcast_ref::<gtk::Button>()
            .and_then(|button| button.label().map(Into::into))
    }
}

// Canonical common grammar first; raw GTK trigger strings are legacy-only.
fn parse_shortcut(value: &str) -> Option<gtk::ShortcutTrigger> {
    match Shortcut::parse(value) {
        Ok(shortcut) => gtk::ShortcutTrigger::parse_string(&shortcut.to_gtk()),
        Err(_) => gtk::ShortcutTrigger::parse_string(value),
    }
}

fn pixels(value: f32) -> i32 {
    assert!(
        value.is_finite() && value >= 0.0 && value <= i32::MAX as f32,
        "invalid GTK spacing/size: {value}"
    );
    value.round() as i32
}

fn valid_key(key: &str) -> bool {
    !key.is_empty() && !key.contains('/')
}

struct Mounted<T: Clone + 'static> {
    item: Keyed<T>,
    row: gtk::ScrolledWindow,
}

struct Rows<T: Clone + 'static> {
    owner: Weak<Inner>,
    id: String,
    container: gtk::Box,
    top: gtk::Box,
    bottom: gtk::Box,
    scroll: gtk::ScrolledWindow,
    source: RefCell<Vec<Keyed<T>>>,
    mounted: RefCell<Vec<Mounted<T>>>,
    viewport: Cell<Viewport>,
    viewport_ready: Cell<bool>,
    binding: Binding<Viewport>,
    template: Template<T>,
    error: RefCell<Option<String>>,
    viewport_error: RefCell<Option<String>>,
    applying: Cell<bool>,
    queued: Cell<bool>,
}

impl<T: Clone + 'static> Rows<T> {
    fn set_start(&self, start: usize, scroll: bool) {
        if !self.viewport_ready.get() {
            return;
        }
        let mut viewport = self.viewport.get();
        viewport.start = start.min(self.source.borrow().len().saturating_sub(viewport.count));
        if viewport == self.viewport.get() && !scroll {
            return;
        }
        self.viewport.set(viewport);
        self.refresh(scroll);
        if self.binding.is_writable() {
            let _ = self.binding.set(viewport);
        }
    }

    fn refresh(&self, scroll: bool) {
        if !self.viewport_ready.get() {
            return;
        }
        if self.applying.replace(true) {
            self.queued.set(true);
            return;
        }
        let Some(inner) = self.owner.upgrade().filter(|inner| !inner.closed.get()) else {
            self.applying.set(false);
            return;
        };
        let view = View(inner);
        loop {
            self.queued.set(false);
            if view.0.closed.get() {
                break;
            }
            let viewport = self.viewport.get();
            let (total, start, desired) = {
                let source = self.source.borrow();
                let start = viewport
                    .range(source.len())
                    .expect("validated viewport")
                    .start;
                (
                    source.len(),
                    start,
                    source
                        .iter()
                        .skip(start)
                        .take(viewport.count)
                        .cloned()
                        .collect::<Vec<_>>(),
                )
            };
            // Retain matching Store identity, not merely matching keys. Reordering does
            // not close the source Store; offscreen bindings/signals are fully detached.
            let mut old = self.mounted.take();
            let mut retained = Vec::new();
            for stale in old.drain(..) {
                if desired.contains(&stale.item) {
                    retained.push(stale);
                } else {
                    view.cleanup(&format!("{}/{}", self.id, stale.item.key));
                    self.container.remove(&stale.row);
                }
            }
            old = retained;
            let mut mounted = Vec::with_capacity(desired.len());
            for item in desired {
                let existing = old.iter().position(|row| row.item == item);
                let row = if let Some(index) = existing {
                    old.remove(index)
                } else {
                    // Same key with a new Store must release the old scope before remount.
                    if let Some(index) = old.iter().position(|row| row.item.key == item.key) {
                        let stale = old.remove(index);
                        view.cleanup(&format!("{}/{}", self.id, stale.item.key));
                        self.container.remove(&stale.row);
                    }
                    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
                    let row = gtk::ScrolledWindow::builder()
                        .hscrollbar_policy(gtk::PolicyType::Never)
                        .vscrollbar_policy(gtk::PolicyType::Never)
                        .child(&body)
                        .build();
                    row.set_widget_name(&item.key);
                    self.container.insert_child_after(&row, Some(&self.top));
                    let prefix = format!("{}/{}", self.id, item.key);
                    view.in_scope(&prefix, || {
                        (self.template)(
                            &view,
                            &Parent::new(&body, ParentKind::Stack),
                            &prefix,
                            item.model.clone(),
                        )
                    });
                    Mounted { item, row }
                };
                row.row.set_height_request(pixels(viewport.row_height));
                row.row.set_vexpand(false);
                mounted.push(row);
            }
            for stale in old {
                view.cleanup(&format!("{}/{}", self.id, stale.item.key));
                self.container.remove(&stale.row);
            }
            let mut previous: gtk::Widget = self.top.clone().upcast();
            for row in &mounted {
                self.container
                    .reorder_child_after(&row.row, Some(&previous));
                previous = row.row.clone().upcast();
            }
            self.container
                .reorder_child_after(&self.bottom, Some(&previous));
            self.top.set_height_request(
                (start as f64 * pixels(viewport.row_height) as f64).min(i32::MAX as f64) as i32,
            );
            self.bottom.set_height_request(
                ((total - start - mounted.len()) as f64 * pixels(viewport.row_height) as f64)
                    .min(i32::MAX as f64) as i32,
            );
            // Bound the viewport independently of content height, including initial allocation.
            self.scroll.set_max_content_height(-1);
            self.scroll.set_min_content_height(
                pixels(viewport.row_height)
                    .saturating_mul(viewport.count.min(i32::MAX as usize) as i32),
            );
            self.scroll
                .set_max_content_height(self.scroll.min_content_height());
            self.scroll
                .set_height_request(self.scroll.min_content_height());
            self.mounted.replace(mounted);
            if scroll {
                let height = pixels(viewport.row_height) as f64;
                let page = viewport.count as f64 * height;
                self.scroll.vadjustment().configure(
                    start as f64 * height,
                    0.0,
                    (total as f64 * height).max(page),
                    height,
                    page,
                    page,
                );
            }
            if !self.queued.get() {
                break;
            }
        }
        self.applying.set(false);
    }
}

impl<T: Clone + 'static> ListControl for Rows<T> {
    fn viewport(&self, start: usize) {
        self.set_start(start, true);
    }
    fn keys(&self) -> Vec<String> {
        self.mounted
            .borrow()
            .iter()
            .map(|row| row.item.key.clone())
            .collect()
    }
    fn error(&self) -> Option<String> {
        self.viewport_error
            .borrow()
            .clone()
            .or_else(|| self.error.borrow().clone())
    }
}

// The View owns this hook, but neither the hook nor its task owns the View or
// executor. Dropping the View aborts its request, restoring the runtime phase.
struct CoordinatedClose {
    executor: Weak<GtkExecutor>,
    session: Session,
    timeout: Duration,
    pending: RefCell<Option<AbortHandle>>,
}

impl CoordinatedClose {
    fn request(self: &Rc<Self>) {
        let Some(executor) = self.executor.upgrade().filter(|e| !e.closed.get()) else {
            return;
        };
        if self.pending.borrow().is_some()
            || self.session.shutdown().phase() != ShutdownPhase::Running
        {
            return;
        }
        let (abort, registration) = AbortHandle::new_pair();
        self.pending.replace(Some(abort));
        let owner = Rc::downgrade(self);
        // Also resets the gate when the executor drops an unpolled task.
        let guard = Subscription::new(move || {
            if let Some(owner) = owner.upgrade() {
                let pending = owner.pending.take();
                drop(pending);
            }
        });
        let shutdown = self.session.shutdown();
        let timeout = self.timeout;
        executor.spawn(Box::pin(async move {
            let _guard = guard;
            // Another View/external caller may win before this first poll.
            // Busy and Cancelled are non-errors: leave all native Views open,
            // release the gate, and allow a later user request to retry.
            let _outcome = Abortable::new(
                async move { shutdown.request_shutdown(timeout).await },
                registration,
            )
            .await;
        }));
    }
}

impl Drop for CoordinatedClose {
    fn drop(&mut self) {
        if let Some(pending) = self.pending.get_mut().take() {
            pending.abort();
        }
    }
}

/// Real GLib-local executor. Completion schedules one-shot handle cleanup;
/// there is no polling timer. Dropping the host aborts and drops pending futures.
pub struct GtkHost {
    executor: Rc<GtkExecutor>,
    sessions: Rc<RefCell<Vec<Session>>>,
    windows: Rc<RefCell<HashMap<String, View>>>,
}

impl GtkHost {
    /// GTK must already be initialized; the caller continues driving its GTK loop.
    pub fn attach() -> Self {
        assert!(
            gtk::is_initialized_main_thread(),
            "GtkHost must attach on the GTK thread"
        );
        let sessions = Rc::new(RefCell::new(Vec::<Session>::new()));
        let executor = Rc::new_cyclic(|owner| GtkExecutor {
            context: glib::MainContext::default(),
            owner: owner.clone(),
            tasks: RefCell::new(HashMap::new()),
            serial: Cell::new(0),
            completed: RefCell::new(Vec::new()),
            reap: RefCell::new(None),
            sessions: Rc::downgrade(&sessions),
            closed: Cell::new(false),
        });
        Self {
            executor,
            sessions,
            windows: Rc::new(RefCell::new(HashMap::new())),
        }
    }
    pub fn executor(&self) -> Rc<dyn Executor> {
        self.executor.clone()
    }
    pub fn attach_view(&self, application: &gtk::Application, view: &View) {
        view.0.window.set_application(Some(application));
        let weak = Rc::downgrade(&self.executor);
        view.on_close(Rc::new(move || {
            if let Some(executor) = weak.upgrade() {
                executor.close();
            }
        }));
    }
    /// Opt in to asynchronous Session shutdown on native close-request. Does not
    /// register/replace any WindowHooks: register native Views and application
    /// hooks separately and retain their tokens. The caller must drive GTK.
    pub fn coordinate_close(
        &self,
        view: &View,
        session: &Session,
        timeout: Duration,
    ) -> Result<(), String> {
        if self.executor.closed.get() || view.0.closed.get() {
            return Err("host or view is closed".into());
        }
        if session.shutdown().phase() != ShutdownPhase::Running {
            return Err("session is shutting down".into());
        }
        if view.0.coordinated_close.borrow().is_some() {
            return Err("view already has coordinated close".into());
        }
        view.0
            .coordinated_close
            .replace(Some(Rc::new(CoordinatedClose {
                executor: Rc::downgrade(&self.executor),
                session: session.clone(),
                timeout,
                pending: RefCell::new(None),
            })));
        Ok(())
    }
    /// Register an actual view with a runtime Session before `UiHost::attach`.
    /// Default hooks confirm immediately and have no application work to drain.
    /// Applications can separately register their own cancellation/drain hooks.
    pub fn register_view(&self, session: &Session, id: &str, view: &View) -> Result<(), String> {
        if self.executor.closed.get() || view.0.closed.get() {
            return Err("host or view is closed".into());
        }
        if self.windows.borrow().contains_key(id) {
            return Err(format!("duplicate GTK window: {id}"));
        }
        let weak = Rc::downgrade(&view.0);
        let token = session.register_window(
            id,
            WindowHooks {
                confirm: Rc::new(|| Box::pin(async { true })),
                cancel: Rc::new(|| {}),
                drain: Rc::new(|| Box::pin(async {})),
                close: Rc::new(move || {
                    if let Some(inner) = weak.upgrade() {
                        View(inner).close();
                    }
                }),
            },
        )?;
        view.keep_at("", token);
        self.windows.borrow_mut().insert(id.into(), view.clone());
        let weak = Rc::downgrade(&self.windows);
        let executor = Rc::downgrade(&self.executor);
        let id = id.to_owned();
        view.on_close(Rc::new(move || {
            if let Some(windows) = weak.upgrade() {
                let removed = windows.borrow_mut().remove(&id);
                drop(removed);
            }
            if let Some(executor) = executor.upgrade() {
                executor.schedule_reap();
            }
        }));
        Ok(())
    }
    pub fn close(&self) {
        for session in self.sessions.take() {
            session.shutdown().close_now();
        }
        for (_, view) in self.windows.take() {
            view.close();
        }
        self.executor.close();
    }
}

impl UiHost for GtkHost {
    fn executor(&self) -> Rc<dyn Executor> {
        self.executor()
    }
    fn attach(&self, session: Session) -> Result<(), String> {
        if self.executor.closed.get() || session.shutdown().phase() != ShutdownPhase::Running {
            return Err("cannot attach a closed host/session".into());
        }
        self.sessions.borrow_mut().push(session);
        let views: Vec<_> = self.windows.borrow().values().cloned().collect();
        for view in views {
            view.present();
        }
        Ok(())
    }
}

impl Drop for GtkHost {
    fn drop(&mut self) {
        self.close();
    }
}

struct GtkExecutor {
    context: glib::MainContext,
    owner: Weak<GtkExecutor>,
    tasks: RefCell<HashMap<u64, glib::JoinHandle<()>>>,
    serial: Cell<u64>,
    completed: RefCell<Vec<u64>>,
    reap: RefCell<Option<glib::SourceId>>,
    sessions: Weak<RefCell<Vec<Session>>>,
    closed: Cell<bool>,
}
impl GtkExecutor {
    fn schedule_reap(&self) {
        if self.closed.get() || self.reap.borrow().is_some() {
            return;
        }
        let owner = self.owner.clone();
        let source = glib::idle_add_local_once(move || {
            if let Some(executor) = owner.upgrade() {
                // Clear before running any destructor: it may spawn, close or iterate GLib.
                executor.reap.take();
                executor.reap_completed();
            }
        });
        self.reap.replace(Some(source));
    }
    fn reap_completed(&self) {
        let completed = self.completed.take();
        let finished: Vec<_> = {
            let mut tasks = self.tasks.borrow_mut();
            completed
                .into_iter()
                .filter_map(|id| tasks.remove(&id))
                .collect()
        };
        // Dropping the last handle can finalize its Source and application future.
        drop(finished);
        if let Some(sessions) = self.sessions.upgrade() {
            let (closed, live): (Vec<_>, Vec<_>) = sessions
                .take()
                .into_iter()
                .partition(|session| session.shutdown().phase() == ShutdownPhase::Closed);
            sessions.borrow_mut().extend(live);
            drop(closed);
        }
    }
    fn close(&self) {
        if self.closed.replace(true) {
            return;
        }
        let source = self.reap.take();
        if let Some(source) = source {
            source.remove();
        }
        self.completed.take();
        for (_, task) in self.tasks.take() {
            task.abort();
            drop(task);
        }
    }
}

// GLib retains a completed future until its Source is finalized. Notify on Ready
// (also on unwind), not merely when that future is eventually dropped.
struct TaskCompletion {
    owner: Weak<GtkExecutor>,
    id: Option<u64>,
}
impl TaskCompletion {
    fn finish(&mut self) {
        if let Some(id) = self.id.take()
            && let Some(executor) = self
                .owner
                .upgrade()
                .filter(|executor| !executor.closed.get())
        {
            executor.completed.borrow_mut().push(id);
            executor.schedule_reap();
        }
    }
}
impl Drop for TaskCompletion {
    fn drop(&mut self) {
        self.finish();
    }
}

impl Drop for GtkExecutor {
    fn drop(&mut self) {
        self.close();
    }
}
impl Executor for GtkExecutor {
    fn spawn(&self, mut future: LocalBoxFuture<'static, ()>) {
        if self.closed.get() {
            return;
        }
        let id = self.serial.get();
        self.serial
            .set(id.checked_add(1).expect("GTK task serial exhausted"));
        let mut completion = TaskCompletion {
            owner: self.owner.clone(),
            id: Some(id),
        };
        let task = self.context.spawn_local(std::future::poll_fn(move |cx| {
            let result = catch_unwind(AssertUnwindSafe(|| future.as_mut().poll(cx)));
            match result {
                Ok(poll) => {
                    if poll.is_ready() {
                        completion.finish();
                    }
                    poll
                }
                Err(panic) => {
                    completion.finish();
                    std::panic::resume_unwind(panic)
                }
            }
        }));
        self.tasks.borrow_mut().insert(id, task);
    }
}

fn panic_error(panic: Box<dyn std::any::Any + Send>) -> String {
    let message = panic
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| panic.downcast_ref::<&str>().copied())
        .unwrap_or("non-string panic");
    format!("GTK callback panicked: {message}")
}

pub fn run(make: impl FnOnce(Rc<dyn Executor>) -> View + 'static) -> Result<(), String> {
    gtk::init().map_err(|error| error.to_string())?;
    let host = Rc::new(GtkHost::attach());
    let app = gtk::Application::new(None::<&str>, gio::ApplicationFlags::NON_UNIQUE);
    app.register(None::<&gio::Cancellable>)
        .map_err(|error| error.to_string())?;
    let owner = Rc::new(RefCell::new(None::<View>));
    let error = Rc::new(RefCell::new(None));
    let make = RefCell::new(Some(make));
    let retained = owner.clone();
    let failure = error.clone();
    let pending = Rc::downgrade(&host);
    let activate = app.connect_activate(move |app| {
        let Some(make) = make.borrow_mut().take() else {
            return;
        };
        let Some(host) = pending.upgrade() else {
            app.quit();
            return;
        };
        match catch_unwind(AssertUnwindSafe(|| make(host.executor()))) {
            Ok(view) => {
                if view.0.closed.get() {
                    app.quit();
                } else {
                    host.attach_view(app, &view);
                    view.present();
                }
                retained.replace(Some(view));
            }
            Err(panic) => {
                failure.replace(Some(panic_error(panic)));
                app.quit();
            }
        }
    });
    let status = app.run_with_args(&["panechord-gtk"]);
    app.disconnect(activate);
    if let Some(view) = owner.take() {
        view.close();
        if let Some(problem) = view.0.error.take() {
            error.replace(Some(problem));
        }
    }
    host.close();
    if let Some(error) = error.take() {
        return Err(error);
    }
    if status != glib::ExitCode::SUCCESS {
        return Err(format!("GTK exited with {status:?}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests;

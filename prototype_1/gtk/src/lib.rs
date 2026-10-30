//! Standalone retained GTK4 adapter. All operations belong to the GTK thread.
//! `run` initializes GTK; manually constructed views require `gtk::init()` first.
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    panic::{AssertUnwindSafe, catch_unwind},
    rc::Rc,
};

use futures::future::LocalBoxFuture;
use gtk::{gio, glib, prelude::*};
use wakui_runtime::{Binding, Command, Executor, ListItem, Subscription};

#[derive(Clone)]
pub struct Parent(gtk::Box);

#[derive(Clone)]
pub struct Menu(gio::Menu);

#[derive(Clone)]
pub struct View(Rc<Inner>);

struct Signal(glib::WeakRef<glib::Object>, Option<glib::SignalHandlerId>);

impl Drop for Signal {
    fn drop(&mut self) {
        if let (Some(object), Some(id)) = (self.0.upgrade(), self.1.take()) {
            object.disconnect(id);
        }
    }
}

struct Rows {
    container: glib::WeakRef<gtk::Box>,
    labels: RefCell<HashMap<String, gtk::Label>>,
    error: RefCell<Option<String>>,
}

impl Rows {
    fn apply(&self, items: Vec<ListItem>, template: fn(&ListItem) -> String) {
        let Some(container) = self.container.upgrade() else {
            return;
        };
        let mut keys = HashSet::new();
        if let Some(duplicate) = items.iter().find(|item| !keys.insert(item.key.clone())) {
            let error = format!(
                "duplicate item key {:?}; keeping last valid rows",
                duplicate.key
            );
            eprintln!("wakui-gtk: {error}");
            container.set_tooltip_text(Some(&error));
            self.error.replace(Some(error));
            return;
        }
        self.error.replace(None);
        container.set_tooltip_text(None);
        // Move the registry out before native setters: no RefCell borrow crosses GTK.
        let mut labels = self.labels.take();
        labels.retain(|key, label| {
            if keys.contains(key) {
                true
            } else {
                container.remove(label);
                false
            }
        });
        let mut previous: Option<gtk::Label> = None;
        for item in items {
            let label = labels.entry(item.key.clone()).or_insert_with(|| {
                let label = gtk::Label::new(None);
                label.set_xalign(0.0);
                label.set_margin_top(6);
                label.set_margin_bottom(6);
                label.set_widget_name(&item.key);
                container.append(&label);
                label
            });
            label.set_text(&template(&item));
            container.reorder_child_after(label, previous.as_ref());
            previous = Some(label.clone());
        }
        self.labels.replace(labels);
    }
}

struct Inner {
    window: gtk::Window,
    root: gtk::Box,
    menus: gio::Menu,
    group: gio::SimpleActionGroup,
    ids: RefCell<HashSet<String>>,
    widgets: RefCell<HashMap<String, gtk::Widget>>,
    actions: RefCell<HashMap<String, gio::SimpleAction>>,
    rows: RefCell<HashMap<String, Rc<Rows>>>,
    subscriptions: RefCell<Vec<Subscription>>,
    signals: RefCell<Vec<Signal>>,
    on_close: RefCell<Option<Rc<dyn Fn()>>>,
    closed: Cell<bool>,
    error: RefCell<Option<String>>,
}

impl Inner {
    fn shutdown(&self) {
        if self.closed.replace(true) {
            return;
        }
        let subscriptions = self.subscriptions.take();
        let signals = self.signals.take();
        let callback = self.on_close.take();
        drop(subscriptions);
        drop(signals);
        if let Some(callback) = callback
            && let Err(panic) = catch_unwind(AssertUnwindSafe(|| callback()))
        {
            self.error.replace(Some(panic_error(panic)));
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
        let root = gtk::Box::new(gtk::Orientation::Vertical, 10);
        root.set_margin_top(16);
        root.set_margin_bottom(16);
        root.set_margin_start(16);
        root.set_margin_end(16);
        root.set_vexpand(true);
        let menus = gio::Menu::new();
        outer.append(&gtk::PopoverMenuBar::from_model(Some(&menus)));
        outer.append(&root);
        window.set_child(Some(&outer));
        let group = gio::SimpleActionGroup::new();
        window.insert_action_group("wakui", Some(&group));
        let view = Self(Rc::new(Inner {
            window,
            root,
            menus,
            group,
            ids: RefCell::default(),
            widgets: RefCell::default(),
            actions: RefCell::default(),
            rows: RefCell::default(),
            subscriptions: RefCell::default(),
            signals: RefCell::default(),
            on_close: RefCell::default(),
            closed: Cell::new(false),
            error: RefCell::default(),
        }));
        let weak = Rc::downgrade(&view.0);
        let signal = view.0.window.connect_close_request(move |_| {
            if let Some(inner) = weak.upgrade() {
                inner.shutdown();
            }
            glib::Propagation::Proceed
        });
        view.signal(&view.0.window, signal);
        view.bind(title, &view.0.window, |window, title| {
            window.set_title(Some(&title))
        });
        view
    }

    fn signal(&self, object: &impl IsA<glib::Object>, id: glib::SignalHandlerId) {
        let signal = Signal(object.as_ref().downgrade(), Some(id));
        if !self.0.closed.get() {
            self.0.signals.borrow_mut().push(signal);
        }
    }

    fn bind<T: Clone + 'static, O: IsA<glib::Object>>(
        &self,
        binding: Binding<T>,
        object: &O,
        apply: impl Fn(&O, T) + 'static,
    ) {
        let weak = object.downgrade();
        // subscribe delivers synchronously; do not borrow the token registry yet.
        let token = binding.subscribe(Rc::new(move |value| {
            if let Some(object) = weak.upgrade() {
                apply(&object, value);
            }
        }));
        if !self.0.closed.get() {
            self.0.subscriptions.borrow_mut().push(token);
        }
    }

    fn reserve(&self, id: &str) {
        assert!(!self.0.closed.get(), "cannot add controls to a closed view");
        assert!(
            self.0.ids.borrow_mut().insert(id.to_owned()),
            "duplicate view id: {id}"
        );
    }

    fn add(&self, parent: &Parent, id: &str, widget: &impl IsA<gtk::Widget>) {
        self.reserve(id);
        widget.set_widget_name(id);
        parent.0.append(widget);
        self.0
            .widgets
            .borrow_mut()
            .insert(id.to_owned(), widget.as_ref().clone());
    }

    pub fn root(&self) -> Parent {
        Parent(self.0.root.clone())
    }

    pub fn panel(&self, parent: &Parent, id: &str, gap: u32) -> Parent {
        let panel = gtk::Box::new(gtk::Orientation::Vertical, gap.min(i32::MAX as u32) as i32);
        self.add(parent, id, &panel);
        Parent(panel)
    }

    pub fn label(&self, parent: &Parent, id: &str, text: Binding<String>) {
        let label = gtk::Label::new(None);
        label.set_xalign(0.0);
        label.set_wrap(true);
        self.add(parent, id, &label);
        self.bind(text, &label, |label, text| label.set_text(&text));
    }

    pub fn entry(&self, parent: &Parent, id: &str, text: Binding<String>) {
        let entry = gtk::Entry::new();
        entry.set_editable(text.is_writable());
        self.add(parent, id, &entry);
        let applying = Rc::new(Cell::new(false));
        let guard = applying.clone();
        let input = text.clone();
        let signal = entry.connect_changed(move |entry| {
            if !guard.get() {
                input.set(entry.text().to_string());
            }
        });
        self.signal(&entry, signal);
        self.bind(text, &entry, move |entry, text| {
            let previous = applying.replace(true);
            if entry.text().as_str() != text {
                entry.set_text(&text);
            }
            applying.set(previous);
        });
    }

    pub fn button(&self, parent: &Parent, id: &str, text: Binding<String>, command: Command) {
        let button = gtk::Button::new();
        self.add(parent, id, &button);
        self.bind(text, &button, |button, text| button.set_label(&text));
        self.bind(command.enabled.clone(), &button, |button, enabled| {
            button.set_sensitive(enabled)
        });
        let signal = button.connect_clicked(move |_| {
            command.invoke();
        });
        self.signal(&button, signal);
    }

    pub fn items(
        &self,
        parent: &Parent,
        id: &str,
        items: Binding<Vec<ListItem>>,
        template: fn(&ListItem) -> String,
    ) {
        let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
        container.add_css_class("boxed-list");
        let scroll = gtk::ScrolledWindow::builder()
            .min_content_height(120)
            .vexpand(true)
            .child(&container)
            .build();
        self.add(parent, id, &scroll);
        let rows = Rc::new(Rows {
            container: container.downgrade(),
            labels: RefCell::default(),
            error: RefCell::default(),
        });
        self.0.rows.borrow_mut().insert(id.to_owned(), rows.clone());
        let weak = Rc::downgrade(&rows);
        self.bind(items, &container, move |_, items| {
            if let Some(rows) = weak.upgrade() {
                rows.apply(items, template);
            }
        });
    }

    pub fn menu(&self, id: &str, title: &str) -> Menu {
        self.reserve(id);
        let menu = gio::Menu::new();
        self.0.menus.append_submenu(Some(title), &menu);
        Menu(menu)
    }

    pub fn menu_item(&self, menu: &Menu, id: &str, text: Binding<String>, command: Command) {
        self.reserve(id);
        // Generated action names accept arbitrary user IDs without GAction syntax restrictions.
        let name = format!("action{}", self.0.actions.borrow().len());
        let action = gio::SimpleAction::new(&name, None);
        self.0.group.add_action(&action);
        self.0
            .actions
            .borrow_mut()
            .insert(id.to_owned(), action.clone());
        self.bind(command.enabled.clone(), &action, |action, enabled| {
            action.set_enabled(enabled)
        });
        let signal = action.connect_activate(move |_, _| {
            command.invoke();
        });
        self.signal(&action, signal);
        let detailed = format!("wakui.{name}");
        let position = menu.0.n_items();
        menu.0.append(Some(""), Some(&detailed));
        self.bind(text, &menu.0, move |menu, text| {
            // GMenu copies items: update this slot only, on a binding notification.
            let item = gio::MenuItem::new(Some(&text), Some(&detailed));
            menu.remove(position);
            menu.insert_item(position, &item);
        });
    }

    /// Retains the callback (and any app-owned model it captures) until close.
    pub fn on_close(&self, callback: Rc<dyn Fn()>) {
        if !self.0.closed.get() {
            let old = self.0.on_close.replace(Some(callback));
            drop(old);
        }
    }

    pub fn close(&self) {
        self.0.shutdown();
        self.0.window.destroy();
    }

    pub fn present(&self) {
        if !self.0.closed.get() {
            self.0.window.present();
        }
    }

    fn widget<W: IsA<gtk::Widget>>(&self, id: &str) -> Result<W, String> {
        if self.0.closed.get() {
            return Err("view is closed".to_owned());
        }
        let widget = self.0.widgets.borrow().get(id).cloned();
        widget
            .and_then(|widget| widget.downcast::<W>().ok())
            .ok_or_else(|| format!("missing or wrong control: {id}"))
    }

    pub fn edit(&self, id: &str, value: &str) -> Result<(), String> {
        let entry = self.widget::<gtk::Entry>(id)?;
        if !entry.is_editable() {
            return Err(format!("read-only entry: {id}"));
        }
        entry.set_text(value);
        Ok(())
    }

    pub fn click(&self, id: &str) -> Result<(), String> {
        let button = self.widget::<gtk::Button>(id)?;
        if !button.is_sensitive() {
            return Err(format!("disabled button: {id}"));
        }
        button.emit_clicked();
        Ok(())
    }

    pub fn activate_menu(&self, id: &str) -> Result<(), String> {
        if self.0.closed.get() {
            return Err("view is closed".to_owned());
        }
        let action = self
            .0
            .actions
            .borrow()
            .get(id)
            .cloned()
            .ok_or_else(|| format!("missing menu item: {id}"))?;
        if !action.is_enabled() {
            return Err(format!("disabled menu item: {id}"));
        }
        action.activate(None);
        Ok(())
    }

    pub fn text(&self, id: &str) -> Option<String> {
        let widget = self.0.widgets.borrow().get(id).cloned()?;
        if let Some(label) = widget.downcast_ref::<gtk::Label>() {
            return Some(label.text().to_string());
        }
        widget
            .downcast_ref::<gtk::Entry>()
            .map(|entry| entry.text().to_string())
    }

    /// Reads native child order, not the last model snapshot.
    pub fn item_keys(&self, id: &str) -> Vec<String> {
        let rows = self.0.rows.borrow().get(id).cloned();
        let mut child = rows
            .and_then(|rows| rows.container.upgrade())
            .and_then(|container| container.first_child());
        let mut keys = Vec::new();
        while let Some(row) = child {
            keys.push(row.widget_name().to_string());
            child = row.next_sibling();
        }
        keys
    }

    /// Process-local identity diagnostic; compare while the row remains alive.
    pub fn row_identity(&self, id: &str, key: &str) -> Option<usize> {
        let rows = self.0.rows.borrow().get(id).cloned()?;
        rows.labels
            .borrow()
            .get(key)
            .map(|row| row.as_ptr() as usize)
    }

    /// Invalid updates leave the last valid rows intact and expose this error.
    pub fn items_error(&self, id: &str) -> Option<String> {
        self.0
            .rows
            .borrow()
            .get(id)
            .and_then(|rows| rows.error.borrow().clone())
    }

    pub fn title(&self) -> String {
        self.0
            .window
            .title()
            .map(|title| title.to_string())
            .unwrap_or_default()
    }
}

struct GtkExecutor {
    context: glib::MainContext,
    tasks: RefCell<Vec<glib::JoinHandle<()>>>,
    closed: Cell<bool>,
}

impl GtkExecutor {
    fn new(context: glib::MainContext) -> Self {
        Self {
            context,
            tasks: RefCell::default(),
            closed: Cell::new(false),
        }
    }

    fn close(&self) {
        if self.closed.replace(true) {
            return;
        }
        let tasks = self.tasks.take();
        for task in tasks {
            task.abort();
            // The handle also owns the Source: drop it here to finalize the local
            // future on this GTK thread, without another main-context iteration.
            // Neither abort nor user Future destructors run under a registry borrow.
            drop(task);
        }
    }
}

impl Drop for GtkExecutor {
    fn drop(&mut self) {
        self.close();
    }
}

impl Executor for GtkExecutor {
    fn spawn(&self, future: LocalBoxFuture<'static, ()>) {
        if self.closed.get() {
            return;
        }
        let task = self.context.spawn_local(future);
        self.tasks.borrow_mut().push(task);
    }
}

struct RunGuard {
    executor: Rc<GtkExecutor>,
    owner: Rc<RefCell<Option<View>>>,
}

impl Drop for RunGuard {
    fn drop(&mut self) {
        let view = self.owner.take();
        if let Some(view) = view {
            view.close();
        }
        self.executor.close();
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

/// Runs a non-unique GTK application, returning normally when its window closes.
pub fn run(make: impl FnOnce(Rc<dyn Executor>) -> View + 'static) -> Result<(), String> {
    gtk::init().map_err(|error| error.to_string())?;
    let app = gtk::Application::new(None::<&str>, gio::ApplicationFlags::NON_UNIQUE);
    app.register(None::<&gio::Cancellable>)
        .map_err(|error| error.to_string())?;
    let owner = Rc::new(RefCell::new(None::<View>));
    let executor = Rc::new(GtkExecutor::new(glib::MainContext::default()));
    let _guard = RunGuard {
        executor: executor.clone(),
        owner: owner.clone(),
    };
    let error = Rc::new(RefCell::new(None));
    let make = RefCell::new(Some(make));
    let retained = owner.clone();
    let failure = error.clone();
    let pending = Rc::downgrade(&executor);
    let activate = app.connect_activate(move |app| {
        let make = make.borrow_mut().take();
        let Some(make) = make else { return };
        let Some(executor) = pending.upgrade() else {
            app.quit();
            return;
        };
        match catch_unwind(AssertUnwindSafe(|| make(executor))) {
            Ok(view) => {
                retained.replace(Some(view.clone()));
                if view.0.closed.get() {
                    app.quit();
                } else {
                    view.0.window.set_application(Some(app));
                    view.present();
                }
            }
            Err(panic) => {
                failure.replace(Some(panic_error(panic)));
                app.quit();
            }
        }
    });
    let status = app.run_with_args(&["wakui-gtk"]);
    app.disconnect(activate);
    let view = owner.take();
    if let Some(view) = view {
        view.close();
        let problem = view.0.error.borrow().clone().or_else(|| {
            view.0
                .rows
                .borrow()
                .values()
                .find_map(|rows| rows.error.borrow().clone())
        });
        if let Some(problem) = problem {
            error.replace(Some(problem));
        }
    }
    executor.close();
    if let Some(error) = error.take() {
        return Err(error);
    }
    if status != glib::ExitCode::SUCCESS {
        return Err(format!("GTK exited with {status:?}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wakui_contract::{CommandKey, Contract, Descriptor, PropertyKey};
    use wakui_runtime::{ModelHandle, UpdateCtx, ViewModel};

    #[derive(Clone)]
    struct State {
        text: String,
        enabled: bool,
        calls: usize,
        writes: usize,
        rows: Vec<ListItem>,
    }
    enum Input {
        Text(String),
        Enable(bool),
        Invoke,
        Rows(Vec<ListItem>),
    }
    struct TestContract;
    impl Contract for TestContract {
        type State = State;
        type Input = Input;
        fn descriptor() -> Descriptor {
            Descriptor {
                name: "gtk-smoke",
                properties: vec![],
                commands: vec![],
            }
        }
    }
    struct TestModel(State);
    impl ViewModel for TestModel {
        type Contract = TestContract;
        type Message = ();
        fn state(&self) -> State {
            self.0.clone()
        }
        fn update(&mut self, input: Input, _: &mut UpdateCtx<()>) {
            match input {
                Input::Text(text) => {
                    self.0.text = text;
                    self.0.writes += 1;
                }
                Input::Enable(enabled) => self.0.enabled = enabled,
                Input::Invoke => {
                    self.0.calls += 1;
                    self.0.text = format!("call {}", self.0.calls);
                }
                Input::Rows(rows) => self.0.rows = rows,
            }
        }
        fn complete(&mut self, _: (), _: &mut UpdateCtx<()>) {}
    }
    static TEXT: PropertyKey<TestContract, String> = PropertyKey {
        name: "text",
        read: |s| s.text.clone(),
        write: Some(Input::Text),
    };
    static ROWS: PropertyKey<TestContract, Vec<ListItem>> = PropertyKey {
        name: "rows",
        read: |s| s.rows.clone(),
        write: None,
    };
    static COMMAND: CommandKey<TestContract> = CommandKey {
        name: "invoke",
        input: || Input::Invoke,
        enabled: |s| s.enabled,
    };
    fn rows(keys: &[&str]) -> Vec<ListItem> {
        keys.iter()
            .map(|key| ListItem {
                key: (*key).into(),
                text: format!("row {key}"),
            })
            .collect()
    }

    #[test]
    fn native_smoke() {
        if let Err(error) = gtk::init() {
            let absent = ["DISPLAY", "WAYLAND_DISPLAY"]
                .iter()
                .all(|key| std::env::var_os(key).is_none_or(|value| value.is_empty()));
            if absent {
                eprintln!("SKIP native_smoke: no display ({error}); run under xvfb");
                return;
            }
            panic!("GTK initialization failed with a configured display: {error}");
        }
        let context = glib::MainContext::default();
        let executor = Rc::new(GtkExecutor::new(context.clone()));
        let polled = Rc::new(Cell::new(false));
        let flag = polled.clone();
        executor.spawn(Box::pin(async move {
            flag.set(true);
        }));
        let task_marker = Rc::new(());
        let weak_task_marker = Rc::downgrade(&task_marker);
        let pending_polled = Rc::new(Cell::new(false));
        let flag = pending_polled.clone();
        executor.spawn(Box::pin(std::future::poll_fn(move |_| {
            let _ = &task_marker;
            flag.set(true);
            std::task::Poll::<()>::Pending
        })));
        while context.pending() {
            context.iteration(false);
        }
        assert!(polled.get());
        assert!(pending_polled.get(), "drop witness must be polled first");
        assert!(weak_task_marker.upgrade().is_some());
        let model = ModelHandle::new(
            TestModel(State {
                text: "initial".into(),
                enabled: true,
                calls: 0,
                writes: 0,
                rows: rows(&["a", "b", "c"]),
            }),
            executor.clone(),
        );
        let view = View::new(model.bind(&TEXT).map(|text| format!("Title {text}")));
        let parent = view.panel(&view.root(), "panel", 8);
        view.label(&parent, "label", model.bind(&TEXT));
        view.entry(&parent, "entry", model.bind(&TEXT));
        view.entry(&parent, "readonly", Binding::constant("fixed".to_owned()));
        assert!(view.edit("readonly", "must not change").is_err());
        assert_eq!(view.text("readonly").as_deref(), Some("fixed"));
        let command = model.command(&COMMAND);
        view.button(&parent, "button", model.bind(&TEXT), command.clone());
        let menu = view.menu("menu", "Actions");
        view.menu_item(&menu, "action", model.bind(&TEXT), command);
        view.items(&parent, "rows", model.bind(&ROWS), |item| item.text.clone());
        view.present();
        while context.pending() {
            context.iteration(false);
        }
        assert_eq!(model.snapshot().writes, 0, "initial binding must not echo");
        view.edit("entry", "typed").unwrap();
        assert_eq!(
            model.snapshot().text,
            "typed",
            "actual Entry changed signal writes back"
        );
        assert_eq!(view.text("label").as_deref(), Some("typed"));
        assert_eq!(view.title(), "Title typed");
        let writes = model.snapshot().writes;
        model.send(Input::Text("from model".into()));
        assert_eq!(view.text("entry").as_deref(), Some("from model"));
        assert_eq!(
            model.snapshot().writes,
            writes + 1,
            "programmatic updates must not echo"
        );
        view.click("button").unwrap();
        view.activate_menu("action").unwrap();
        assert_eq!(model.snapshot().calls, 2);
        assert_eq!(view.text("label").as_deref(), Some("call 2"));
        assert_eq!(
            menu.0.item_attribute_value(0, "label", None).unwrap().str(),
            Some("call 2")
        );
        assert_eq!(
            view.widget::<gtk::Button>("button")
                .unwrap()
                .label()
                .as_deref(),
            Some("call 2")
        );
        model.send(Input::Enable(false));
        assert!(!view.widget::<gtk::Button>("button").unwrap().is_sensitive());
        assert!(!view.0.actions.borrow()["action"].is_enabled());
        assert!(view.click("button").is_err());
        assert!(view.activate_menu("action").is_err());
        assert_eq!(model.snapshot().calls, 2);
        model.send(Input::Enable(true));
        view.activate_menu("action").unwrap();
        assert_eq!(model.snapshot().calls, 3);
        let a = view.row_identity("rows", "a").unwrap();
        let b = view.row_identity("rows", "b").unwrap();
        model.send(Input::Rows(rows(&["b", "a", "d"])));
        assert_eq!(view.item_keys("rows"), ["b", "a", "d"]);
        assert_eq!(view.row_identity("rows", "a"), Some(a));
        assert_eq!(view.row_identity("rows", "b"), Some(b));
        assert_eq!(view.row_identity("rows", "c"), None);
        model.send(Input::Rows(rows(&["a", "a"])));
        assert!(view.items_error("rows").unwrap().contains("duplicate"));
        assert_eq!(view.item_keys("rows"), ["b", "a", "d"]);
        model.send(Input::Rows(rows(&["a", "b"])));
        assert!(view.items_error("rows").is_none());
        let closed = Rc::new(Cell::new(0));
        let count = closed.clone();
        let marker = Rc::new(());
        let weak_marker = Rc::downgrade(&marker);
        let owner = model.clone();
        view.on_close(Rc::new(move || {
            let _ = (&owner, &marker);
            count.set(count.get() + 1);
        }));
        let entry = view.widget::<gtk::Entry>("entry").unwrap();
        let button = view.widget::<gtk::Button>("button").unwrap();
        view.0.window.close(); // Real window-manager close-request path.
        assert_eq!(closed.get(), 1, "native close-request must notify");
        view.close();
        view.close();
        assert_eq!(closed.get(), 1);
        assert!(
            weak_marker.upgrade().is_none(),
            "close releases its captured owner"
        );
        assert!(view.0.subscriptions.borrow().is_empty());
        assert!(view.0.signals.borrow().is_empty());
        let frozen = view.text("label");
        let title = view.title();
        model.send(Input::Text("after close".into()));
        model.send(Input::Rows(vec![]));
        assert_eq!(view.text("label"), frozen, "subscriptions must be detached");
        assert_eq!(view.title(), title);
        assert_eq!(view.item_keys("rows"), ["a", "b"]);
        assert!(view.edit("entry", "closed").is_err());
        entry.set_text("detached input");
        button.emit_clicked();
        assert_eq!(model.snapshot().text, "after close");
        assert_eq!(model.snapshot().calls, 3);
        let weak = Rc::downgrade(&view.0);
        drop(view);
        assert!(
            weak.upgrade().is_none(),
            "native signals must not retain View"
        );
        executor.close();
        assert!(
            weak_task_marker.upgrade().is_none(),
            "close must drop pending futures without another native iteration"
        );
        executor.close();
        assert!(executor.tasks.borrow().is_empty());
        let rejected_marker = Rc::new(());
        let weak_rejected_marker = Rc::downgrade(&rejected_marker);
        let rejected_polled = Rc::new(Cell::new(false));
        let flag = rejected_polled.clone();
        executor.spawn(Box::pin(std::future::poll_fn(move |_| {
            let _ = &rejected_marker;
            flag.set(true);
            std::task::Poll::<()>::Pending
        })));
        assert!(
            weak_rejected_marker.upgrade().is_none(),
            "a closed executor must immediately drop new futures"
        );
        while context.pending() {
            context.iteration(false);
        }
        assert!(
            !rejected_polled.get(),
            "rejected tasks must never be polled"
        );
    }
}

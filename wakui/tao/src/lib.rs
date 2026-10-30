//! Independent Tao + iced/tiny-skia adapter. Construct and use on the UI thread.
//!
//! Automation dispatches the same messages as iced controls and native muda
//! actions; unlike GTK automation it does NOT emit native widget signals.
//! Lists support stateless label templates only: keys/order are retained, but
//! individual iced label trees have no observable native widget identity.
//! Clipboard is application-local; system clipboard and IME preedit are not
//! integrated. Unknown physical/non-editing keys are reported as unidentified.
//! `run` owns one desktop event loop and returns normally (no process exit).
mod driver;
mod input;
mod scheduler;

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    panic::{AssertUnwindSafe, catch_unwind},
    rc::{Rc, Weak},
};

use iced::{Length, widget};
use wakui_runtime::{Binding, Command, Executor, ListItem, Subscription};

type Element = iced::Element<'static, Message, iced::Theme, iced_tiny_skia::Renderer>;
type Part = Rc<dyn Fn() -> Element>;

#[derive(Clone)]
pub struct Parent(Rc<Panel>);
#[derive(Clone)]
pub struct Menu(muda::Submenu, Weak<Inner>);
#[derive(Clone)]
pub struct View(Rc<Inner>);

struct Panel {
    owner: Weak<Inner>,
    gap: u32,
    parts: RefCell<Vec<Part>>,
}
impl Panel {
    fn element(&self) -> Element {
        let parts = self.parts.borrow().clone();
        widget::Column::with_children(parts.iter().map(|part| part()))
            .spacing(self.gap as f32)
            .width(Length::Fill)
            .into()
    }
}

#[derive(Clone, Debug)]
enum Message {
    Edit(String, String),
    Click(String),
    Menu(String),
}
#[derive(Clone)]
enum Handler {
    Entry(Binding<String>),
    Button(Command),
    Menu(Command),
}
#[derive(Default)]
struct Rows {
    values: RefCell<Vec<(String, String)>>,
}

struct Inner {
    root: Parent,
    title: RefCell<String>,
    ids: RefCell<HashSet<String>>,
    texts: RefCell<HashMap<String, Rc<RefCell<String>>>>,
    rows: RefCell<HashMap<String, Rc<Rows>>>,
    handlers: RefCell<HashMap<String, Handler>>,
    menu_ids: RefCell<HashMap<muda::MenuId, String>>,
    menus: muda::Menu,
    subscriptions: RefCell<Vec<Subscription>>,
    on_close: RefCell<Option<Rc<dyn Fn()>>>,
    invalidate: RefCell<Option<Rc<dyn Fn()>>>,
    visible: Cell<bool>,
    closed: Cell<bool>,
    error: RefCell<Option<String>>,
}

impl Inner {
    fn refresh(&self) {
        let callback = self.invalidate.borrow().clone();
        if let Some(callback) = callback {
            callback();
        }
    }
    fn shutdown(&self) {
        if self.closed.replace(true) {
            return;
        }
        let subscriptions = self.subscriptions.take();
        let handlers = self.handlers.take();
        let callback = self.on_close.take();
        drop(subscriptions);
        drop(handlers);
        self.refresh();
        drop(self.invalidate.take());
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
    }
}

impl View {
    pub fn new(title: Binding<String>) -> Self {
        let view = Self(Rc::new_cyclic(|owner| Inner {
            root: Parent(Rc::new(Panel {
                owner: owner.clone(),
                gap: 10,
                parts: RefCell::default(),
            })),
            title: RefCell::default(),
            ids: RefCell::default(),
            texts: RefCell::default(),
            rows: RefCell::default(),
            handlers: RefCell::default(),
            menu_ids: RefCell::default(),
            menus: muda::Menu::new(),
            subscriptions: RefCell::default(),
            on_close: RefCell::default(),
            invalidate: RefCell::default(),
            visible: Cell::new(false),
            closed: Cell::new(false),
            error: RefCell::default(),
        }));
        let weak = Rc::downgrade(&view.0);
        view.bind(title, move |title| {
            if let Some(inner) = weak.upgrade() {
                inner.title.replace(title);
            }
        });
        view
    }

    fn bind<T: Clone + 'static>(&self, binding: Binding<T>, apply: impl Fn(T) + 'static) {
        let weak = Rc::downgrade(&self.0);
        let token = binding.subscribe(Rc::new(move |value| {
            if let Some(inner) = weak.upgrade().filter(|inner| !inner.closed.get()) {
                apply(value);
                inner.refresh();
            }
        }));
        if !self.0.closed.get() {
            self.0.subscriptions.borrow_mut().push(token);
        }
    }
    fn reserve(&self, id: &str) {
        assert!(!self.0.closed.get(), "cannot add controls to a closed view");
        assert!(
            self.0.ids.borrow_mut().insert(id.into()),
            "duplicate view id: {id}"
        );
    }
    fn add(&self, parent: &Parent, id: &str, part: Part) {
        assert!(
            Weak::ptr_eq(&parent.0.owner, &Rc::downgrade(&self.0)),
            "foreign parent"
        );
        self.reserve(id);
        parent.0.parts.borrow_mut().push(part);
        self.0.refresh();
    }
    fn bound_text(&self, id: &str, binding: Binding<String>) -> Rc<RefCell<String>> {
        let value = Rc::new(RefCell::new(String::new()));
        self.0.texts.borrow_mut().insert(id.into(), value.clone());
        let out = value.clone();
        self.bind(binding, move |text| {
            out.replace(text);
        });
        value
    }
    pub fn root(&self) -> Parent {
        self.0.root.clone()
    }
    pub fn panel(&self, parent: &Parent, id: &str, gap: u32) -> Parent {
        let panel = Parent(Rc::new(Panel {
            owner: Rc::downgrade(&self.0),
            gap,
            parts: RefCell::default(),
        }));
        let out = panel.clone();
        self.add(parent, id, Rc::new(move || out.0.element()));
        panel
    }
    pub fn label(&self, parent: &Parent, id: &str, text: Binding<String>) {
        let value = self.bound_text(id, text);
        self.add(
            parent,
            id,
            Rc::new(move || widget::text(value.borrow().clone()).into()),
        );
    }
    pub fn entry(&self, parent: &Parent, id: &str, text: Binding<String>) {
        let editable = text.is_writable();
        let value = self.bound_text(id, text.clone());
        self.0
            .handlers
            .borrow_mut()
            .insert(id.into(), Handler::Entry(text));
        let id = id.to_owned();
        let key = id.clone();
        self.add(
            parent,
            &key,
            Rc::new(move || {
                let id = id.clone();
                let input = widget::text_input("", &value.borrow());
                if editable {
                    input
                        .on_input(move |text| Message::Edit(id.clone(), text))
                        .into()
                } else {
                    input.into()
                }
            }),
        );
    }
    pub fn button(&self, parent: &Parent, id: &str, text: Binding<String>, command: Command) {
        let value = self.bound_text(id, text);
        let enabled = Rc::new(Cell::new(false));
        let out = enabled.clone();
        self.bind(command.enabled.clone(), move |value| out.set(value));
        self.0
            .handlers
            .borrow_mut()
            .insert(id.into(), Handler::Button(command));
        let key = id.to_owned();
        let id = key.clone();
        self.add(
            parent,
            &key,
            Rc::new(move || {
                widget::button(widget::text(value.borrow().clone()))
                    .on_press_maybe(enabled.get().then(|| Message::Click(id.clone())))
                    .into()
            }),
        );
    }
    pub fn items(
        &self,
        parent: &Parent,
        id: &str,
        items: Binding<Vec<ListItem>>,
        template: fn(&ListItem) -> String,
    ) {
        eprintln!(
            "wakui-tao: list {id} uses stateless label rows; native row identity is unavailable"
        );
        let rows = Rc::new(Rows::default());
        let out = rows.clone();
        self.add(
            parent,
            id,
            Rc::new(move || {
                let values = out.values.borrow().clone();
                widget::scrollable(
                    widget::Column::with_children(
                        values
                            .into_iter()
                            .map(|(_, text)| widget::text(text).into()),
                    )
                    .spacing(6),
                )
                .height(Length::Fixed(160.0))
                .width(Length::Fill)
                .into()
            }),
        );
        self.0.rows.borrow_mut().insert(id.into(), rows.clone());
        let weak = Rc::downgrade(&self.0);
        let id = id.to_owned();
        self.bind(items, move |items| {
            let mut keys = HashSet::new();
            if let Some(item) = items.iter().find(|item| !keys.insert(item.key.clone())) {
                let error = format!(
                    "list {id}: duplicate key {:?}; keeping last valid label rows",
                    item.key
                );
                eprintln!("wakui-tao: {error}");
                if let Some(inner) = weak.upgrade() {
                    inner.error.replace(Some(error));
                }
                return;
            }
            // Templates run before borrowing display state. Label rows are stateless.
            let values = items
                .iter()
                .map(|item| (item.key.clone(), template(item)))
                .collect();
            rows.values.replace(values);
        });
    }
    pub fn menu(&self, id: &str, title: &str) -> Menu {
        self.reserve(id);
        let menu = muda::Submenu::new(title, true);
        if let Err(error) = self.0.menus.append(&menu) {
            self.0.error.replace(Some(error.to_string()));
        }
        Menu(menu, Rc::downgrade(&self.0))
    }
    pub fn menu_item(&self, menu: &Menu, id: &str, text: Binding<String>, command: Command) {
        assert!(
            Weak::ptr_eq(&menu.1, &Rc::downgrade(&self.0)),
            "foreign menu"
        );
        self.reserve(id);
        let item = muda::MenuItem::new("", false, None);
        self.0
            .menu_ids
            .borrow_mut()
            .insert(item.id().clone(), id.into());
        if let Err(error) = menu.0.append(&item) {
            self.0.error.replace(Some(error.to_string()));
        }
        let out = item.clone();
        self.bind(text, move |text| out.set_text(text));
        self.bind(command.enabled.clone(), move |enabled| {
            item.set_enabled(enabled)
        });
        self.0
            .handlers
            .borrow_mut()
            .insert(id.into(), Handler::Menu(command));
    }
    /// Retains the application-owned model until close. Usually closes that model too.
    pub fn on_close(&self, callback: Rc<dyn Fn()>) {
        if !self.0.closed.get() {
            let old = self.0.on_close.replace(Some(callback));
            drop(old);
        }
    }
    pub fn close(&self) {
        self.0.shutdown();
    }
    pub fn present(&self) {
        if !self.0.closed.get() {
            self.0.visible.set(true);
            self.0.refresh();
        }
    }
    fn dispatch(&self, message: Message) -> Result<(), String> {
        if self.0.closed.get() {
            return Err("view is closed".into());
        }
        let id = match &message {
            Message::Edit(id, _) | Message::Click(id) | Message::Menu(id) => id.clone(),
        };
        // Clone before calling any model code: notifications may reenter this view.
        let handler = self.0.handlers.borrow().get(&id).cloned();
        match (handler, message) {
            (Some(Handler::Entry(binding)), Message::Edit(id, value)) => {
                let display = self.0.texts.borrow().get(&id).cloned().unwrap();
                if *display.borrow() != value {
                    if !binding.set(value) {
                        return Err(format!("read-only or closed entry: {id}"));
                    }
                    // The model is canonical; normalization/rejection may retain its old value.
                    display.replace(binding.get());
                    self.0.refresh();
                }
            }
            (Some(Handler::Button(command)), Message::Click(id))
            | (Some(Handler::Menu(command)), Message::Menu(id)) => {
                if !command.invoke() {
                    return Err(format!("disabled or closed command: {id}"));
                }
            }
            _ => return Err(format!("missing or wrong control: {id}")),
        }
        Ok(())
    }
    pub fn edit(&self, id: &str, value: &str) -> Result<(), String> {
        self.dispatch(Message::Edit(id.into(), value.into()))
    }
    pub fn click(&self, id: &str) -> Result<(), String> {
        self.dispatch(Message::Click(id.into()))
    }
    pub fn activate_menu(&self, id: &str) -> Result<(), String> {
        self.dispatch(Message::Menu(id.into()))
    }
    pub fn text(&self, id: &str) -> Option<String> {
        self.0
            .texts
            .borrow()
            .get(id)
            .map(|value| value.borrow().clone())
    }
    pub fn item_keys(&self, id: &str) -> Vec<String> {
        self.0
            .rows
            .borrow()
            .get(id)
            .map(|rows| {
                rows.values
                    .borrow()
                    .iter()
                    .map(|(key, _)| key.clone())
                    .collect()
            })
            .unwrap_or_default()
    }
    pub fn title(&self) -> String {
        self.0.title.borrow().clone()
    }
}

fn panic_error(panic: Box<dyn std::any::Any + Send>) -> String {
    let message = panic
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| panic.downcast_ref::<&str>().copied())
        .unwrap_or("non-string panic");
    format!("Tao callback panicked: {message}")
}

/// Must be called on the desktop main thread, with no GTK4 initialized in this process.
/// Tao may only be initialized once per process; run_return is called once, not polled.
/// Windows/macOS native resize operations can suspend outside-loop code (Tao restriction).
/// Owns muda's process-global event handler; do not install a competing handler.
pub fn run(make: impl FnOnce(Rc<dyn Executor>) -> View + 'static) -> Result<(), String> {
    driver::run(make)
}

//! Retained, software-rendered iced controls in a native Tao window and muda menu.
//! See README.md for input and external-reactor boundaries.
mod controls;
mod driver;
mod input;
mod layout;
mod menu;
mod scheduler;
#[cfg(test)]
mod tests;

use iced::{Length, widget};
use panechord_runtime::{
    Binding, Command, Executor, Keyed, Layout, Store, Style, Subscription, Viewport,
};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    panic::{AssertUnwindSafe, catch_unwind},
    rc::{Rc, Weak},
};

/// Application primitives may return any iced widget, including advanced widgets.
pub type Element = iced::Element<'static, Message, iced::Theme, iced_tiny_skia::Renderer>;
pub type ElementFactory = Rc<dyn Fn() -> Element>;
type Template<T> = Rc<dyn Fn(&View, &Parent, &str, Store<T>)>;

#[derive(Clone, Debug)]
pub enum Message {
    Edit(String, String),
    Click(String),
    Menu(String),
    Scroll(String, f32),
    /// Application primitive callback; dispatched outside widget-tree borrows.
    Custom(String),
    Toggle(String, bool),
    Slide(String, f64),
    Select(String, String),
    Open(String, bool),
    Ignored,
}
#[derive(Clone)]
pub struct View(Rc<Inner>);
/// Non-owning UI-thread handle; upgrade rejects closed and dropped Views.
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
#[derive(Clone)]
pub struct Parent(Rc<Node>);
#[derive(Clone)]
enum Handler {
    Entry(Binding<String>),
    Button(Command),
    Toggle(Binding<bool>),
    Radio(Binding<String>, String),
    Slider(Binding<f64>, panechord_runtime::SliderRange, Rc<Cell<f64>>),
    Choice(Binding<String>, Binding<Vec<panechord_runtime::Choice>>),
    Tab(Binding<String>),
    Dialog(Binding<bool>),
}
enum Kind {
    Panel(Layout),
    Leaf(ElementFactory),
    Items(Rc<dyn ItemHost>),
    Content,
    Tab(Rc<controls::Tab>),
    Page,
    Dialog(Rc<controls::Dialog>),
}
struct Node {
    id: String,
    scope: String,
    mounted: Cell<bool>,
    visible: Cell<bool>,
    enabled: Cell<bool>,
    owner: Weak<Inner>,
    parent: Weak<Node>,
    style: Cell<Style>,
    kind: RefCell<Kind>,
    children: RefCell<Vec<Rc<Node>>>,
    subscriptions: RefCell<Vec<Subscription>>,
    text: RefCell<Option<Rc<RefCell<String>>>>,
    handler: RefCell<Option<Handler>>,
    custom: RefCell<Option<Rc<dyn Fn()>>>,
    identity: usize,
}
impl Node {
    fn interactive(&self) -> bool {
        self.mounted.get()
            && self.visible.get()
            && self.enabled.get()
            && self
                .parent
                .upgrade()
                .is_none_or(|parent| parent.interactive())
    }
    fn key(&self) -> String {
        format!("{}#{}", self.id, self.identity)
    }
    fn retire(&self) {
        if !self.mounted.replace(false) {
            return;
        }
        for child in self.children.take() {
            child.retire();
        }
        drop(self.subscriptions.take());
        self.handler.take();
        self.custom.take();
        self.kind.replace(Kind::Content);
        if let Some(inner) = self.owner.upgrade() {
            inner.scroll.borrow_mut().remove(&self.id);
        }
    }
    fn element(&self) -> Element {
        let body = match &*self.kind.borrow() {
            Kind::Panel(policy) => {
                let children = self.children.borrow().clone();
                layout::Panel::element(
                    *policy,
                    children
                        .iter()
                        .map(|n| (n.key(), n.style.get(), n.element()))
                        .collect(),
                )
            }
            Kind::Leaf(factory) => factory(),
            Kind::Content | Kind::Page => layout::Panel::element(
                Layout::Stack {
                    horizontal: false,
                    gap: 0.0,
                },
                self.children
                    .borrow()
                    .iter()
                    .map(|n| (n.key(), n.style.get(), n.element()))
                    .collect(),
            ),
            Kind::Items(host) => host.element(self),
            Kind::Tab(tab) => tab.element(self),
            Kind::Dialog(_) => widget::Space::new().width(0).height(0).into(),
        };
        let p = self.style.get().padding;
        layout::Panel::appearance_frame(
            body,
            iced::Padding {
                top: layout::finite(p.top),
                right: layout::finite(p.right),
                bottom: layout::finite(p.bottom),
                left: layout::finite(p.left),
            },
            self.visible.get(),
            self.enabled.get(),
        )
    }
}
struct Inner {
    root: Parent,
    title: RefCell<String>,
    nodes: RefCell<HashMap<String, Weak<Node>>>,
    next: Cell<usize>,
    frames: Cell<usize>,
    menu_nodes: RefCell<Vec<Rc<Node>>>,
    template_scopes: RefCell<Vec<Weak<Node>>>,
    menus: muda::Menu,
    menu_ids: RefCell<HashMap<muda::MenuId, String>>,
    menu_actions: RefCell<HashMap<String, menu::Action>>,
    subscriptions: RefCell<Vec<Subscription>>,
    on_close: RefCell<Vec<Rc<dyn Fn()>>>,
    on_close_request: RefCell<Option<Rc<dyn Fn()>>>,
    quit_request: RefCell<Option<Rc<dyn Fn()>>>,
    requesting_close: Cell<bool>,
    invalidate: RefCell<Option<Rc<dyn Fn()>>>,
    visible: Cell<bool>,
    closed: Cell<bool>,
    image_updates: Cell<usize>,
    error: RefCell<Option<String>>,
    fatal_error: RefCell<Option<String>>,
    diagnostics: RefCell<Vec<String>>,
    scroll: RefCell<HashMap<String, f32>>,
}
impl Inner {
    fn refresh(&self) {
        if let Some(f) = self.invalidate.borrow().clone() {
            f();
        }
    }
    fn report(&self, error: String) {
        self.diagnostics.borrow_mut().push(error.clone());
        self.error.replace(Some(error));
    }
    fn shutdown(&self) {
        if self.closed.replace(true) {
            return;
        }
        drop(self.on_close_request.take());
        drop(self.quit_request.take());
        drop(self.subscriptions.take());
        // Drop widget subscriptions even if application retains Parent handles.
        let nodes: Vec<_> = self
            .nodes
            .borrow()
            .values()
            .filter_map(Weak::upgrade)
            .collect();
        for node in nodes {
            node.retire();
        }
        self.menu_nodes.borrow_mut().clear();
        self.menu_actions.borrow_mut().clear();
        self.refresh();
        self.invalidate.take();
        for callback in self.on_close.take() {
            if let Err(panic) = catch_unwind(AssertUnwindSafe(|| callback())) {
                self.report(panic_error(panic));
            }
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
            root: Parent(Rc::new(Node {
                id: "__root".into(),
                scope: String::new(),
                mounted: Cell::new(true),
                visible: Cell::new(true),
                enabled: Cell::new(true),
                owner: owner.clone(),
                parent: Weak::new(),
                style: Cell::new(Style::default()),
                kind: RefCell::new(Kind::Panel(Layout::Stack {
                    horizontal: false,
                    gap: 0.0,
                })),
                children: RefCell::default(),
                subscriptions: RefCell::default(),
                text: RefCell::default(),
                handler: RefCell::default(),
                custom: RefCell::default(),
                identity: 0,
            })),
            title: RefCell::default(),
            nodes: RefCell::default(),
            next: Cell::new(1),
            frames: Cell::new(0),
            menu_nodes: RefCell::default(),
            template_scopes: RefCell::default(),
            menus: muda::Menu::new(),
            menu_ids: RefCell::default(),
            menu_actions: RefCell::default(),
            subscriptions: RefCell::default(),
            on_close: RefCell::default(),
            on_close_request: RefCell::default(),
            quit_request: RefCell::default(),
            requesting_close: Cell::new(false),
            invalidate: RefCell::default(),
            visible: Cell::new(false),
            closed: Cell::new(false),
            image_updates: Cell::new(0),
            error: RefCell::default(),
            fatal_error: RefCell::default(),
            diagnostics: RefCell::default(),
            scroll: RefCell::default(),
        }));
        view.0
            .nodes
            .borrow_mut()
            .insert("__root".into(), Rc::downgrade(&view.0.root.0));
        let weak = Rc::downgrade(&view.0);
        let token = title.subscribe(Rc::new(move |value| {
            if let Some(inner) = weak.upgrade().filter(|i| !i.closed.get()) {
                inner.title.replace(value);
                inner.refresh();
            }
        }));
        view.keep(token);
        view
    }
    fn find(&self, id: &str) -> Option<Rc<Node>> {
        self.0
            .nodes
            .borrow()
            .get(id)
            .and_then(Weak::upgrade)
            .filter(|n| n.mounted.get())
    }
    fn add(&self, parent: &Parent, id: &str, kind: Kind) -> Rc<Node> {
        self.add_scoped(parent, id, kind, false)
    }
    fn add_scoped(&self, parent: &Parent, id: &str, kind: Kind, new_scope: bool) -> Rc<Node> {
        assert!(!self.0.closed.get(), "closed view");
        assert!(parent.0.mounted.get(), "evicted parent");
        assert!(
            Weak::ptr_eq(&parent.0.owner, &Rc::downgrade(&self.0)),
            "foreign parent"
        );
        let scope = &parent.0.scope;
        let id = if scope.is_empty() || id.starts_with(&format!("{scope}/")) {
            id.to_owned()
        } else {
            format!("{scope}/{id}")
        };
        assert!(self.find(&id).is_none(), "duplicate view id: {id}");
        let identity = self.0.next.get();
        self.0
            .next
            .set(identity.checked_add(1).expect("mount ID exhausted"));
        let node = Rc::new(Node {
            id: id.clone(),
            scope: if new_scope { id.clone() } else { scope.clone() },
            mounted: Cell::new(true),
            visible: Cell::new(true),
            enabled: Cell::new(true),
            owner: Rc::downgrade(&self.0),
            parent: Rc::downgrade(&parent.0),
            style: Cell::new(Style::default()),
            kind: RefCell::new(kind),
            children: RefCell::default(),
            subscriptions: RefCell::default(),
            text: RefCell::default(),
            handler: RefCell::default(),
            custom: RefCell::default(),
            identity,
        });
        // Discard dead registry entries: scrolling through a large source stays bounded.
        self.0
            .nodes
            .borrow_mut()
            .retain(|_, n| n.upgrade().is_some_and(|n| n.mounted.get()));
        self.0.nodes.borrow_mut().insert(id, Rc::downgrade(&node));
        parent.0.children.borrow_mut().push(node.clone());
        self.0.refresh();
        node
    }
    fn bind<T: Clone + 'static>(
        &self,
        node: &Rc<Node>,
        binding: Binding<T>,
        apply: impl Fn(T) + 'static,
    ) {
        let weak = Rc::downgrade(&self.0);
        let target = Rc::downgrade(node);
        let token = binding.subscribe(Rc::new(move |value| {
            if let (Some(inner), Some(_node)) = (
                weak.upgrade().filter(|i| !i.closed.get()),
                target.upgrade().filter(|n| n.mounted.get()),
            ) {
                apply(value);
                inner.refresh();
            }
        }));
        if !self.0.closed.get() {
            node.subscriptions.borrow_mut().push(token);
        }
    }
    fn bound_text(&self, node: &Rc<Node>, binding: Binding<String>) -> Rc<RefCell<String>> {
        let value = Rc::new(RefCell::new(binding.get()));
        node.text.replace(Some(value.clone()));
        let out = value.clone();
        self.bind(node, binding, move |v| {
            out.replace(v);
        });
        value
    }
    pub fn root(&self) -> Parent {
        self.0.root.clone()
    }
    pub fn layout(&self, parent: &Parent, id: &str, layout: Layout) -> Parent {
        self.try_layout(parent, id, layout).expect("invalid layout")
    }
    pub fn try_layout(&self, parent: &Parent, id: &str, layout: Layout) -> Result<Parent, String> {
        layout.validate().map_err(str::to_owned)?;
        if self.0.closed.get() || !parent.0.mounted.get() || id.is_empty() {
            return Err("closed view, unavailable parent or empty layout id".into());
        }
        if !parent.0.owner.ptr_eq(&Rc::downgrade(&self.0)) || self.find(id).is_some() {
            return Err("foreign parent or duplicate layout id".into());
        }
        Ok(Parent(self.add(parent, id, Kind::Panel(layout))))
    }
    pub fn style(&self, id: &str, style: Style) {
        if let Err(error) = self.try_style(id, style) {
            self.0.report(error);
        }
    }
    pub fn try_style(&self, id: &str, style: Style) -> Result<(), String> {
        if self.0.closed.get() {
            return Err("view is closed".into());
        }
        let node = self
            .find(id)
            .ok_or_else(|| format!("style target not mounted: {id}"))?;
        let parent = node.parent.upgrade();
        let layout = parent
            .as_ref()
            .and_then(|parent| match *parent.kind.borrow() {
                Kind::Panel(layout) => Some(layout),
                _ => None,
            })
            .unwrap_or(Layout::Stack {
                horizontal: false,
                gap: 0.0,
            });
        style.validate(layout).map_err(str::to_owned)?;
        node.style.set(style);
        self.0.refresh();
        Ok(())
    }
    pub fn label(&self, parent: &Parent, id: &str, text: Binding<String>) {
        let node = self.add(parent, id, Kind::Content);
        let value = self.bound_text(&node, text);
        node.kind.replace(Kind::Leaf(Rc::new(move || {
            widget::text(value.borrow().clone())
                .wrapping(iced::widget::text::Wrapping::WordOrGlyph)
                .into()
        })));
    }
    pub fn entry(&self, parent: &Parent, id: &str, text: Binding<String>) {
        let node = self.add(parent, id, Kind::Content);
        let value = self.bound_text(&node, text.clone());
        let editable = text.is_writable();
        let key = node.id.clone();
        node.handler.replace(Some(Handler::Entry(text)));
        node.kind.replace(Kind::Leaf(Rc::new(move || {
            let key = key.clone();
            let input = widget::text_input("", &value.borrow());
            if editable {
                input
                    .on_input(move |v| Message::Edit(key.clone(), v))
                    .into()
            } else {
                input.into()
            }
        })));
    }
    pub fn button(&self, parent: &Parent, id: &str, text: Binding<String>, command: Command) {
        let node = self.add(parent, id, Kind::Content);
        let value = self.bound_text(&node, text);
        let enabled = Rc::new(Cell::new(command.enabled.get()));
        let out = enabled.clone();
        self.bind(&node, command.enabled.clone(), move |v| out.set(v));
        node.handler.replace(Some(Handler::Button(command)));
        let key = node.id.clone();
        node.kind.replace(Kind::Leaf(Rc::new(move || {
            widget::button(
                widget::text(value.borrow().clone())
                    .align_x(iced::alignment::Horizontal::Center)
                    .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
            )
            .on_press_maybe(enabled.get().then(|| Message::Click(key.clone())))
            .into()
        })));
    }
    /// Genuine extension point for application-defined iced/advanced widgets.
    /// `subscriptions` are owned by this mount and dropped when its row is evicted.
    pub fn mount(
        &self,
        parent: &Parent,
        id: &str,
        factory: ElementFactory,
        subscriptions: Vec<Subscription>,
    ) {
        let node = self.add(parent, id, Kind::Leaf(factory));
        node.subscriptions.replace(subscriptions);
    }
    pub fn on_custom(&self, id: &str, callback: Rc<dyn Fn()>) -> Result<(), String> {
        let node = self
            .find(id)
            .ok_or_else(|| format!("missing primitive: {id}"))?;
        node.custom.replace(Some(callback));
        Ok(())
    }
    pub fn meter(&self, parent: &Parent, id: &str, value: Binding<f64>) {
        let node = self.add(parent, id, Kind::Content);
        let current = Rc::new(Cell::new(0.0f32));
        let out = current.clone();
        self.bind(&node, value, move |v| {
            out.set(if v.is_finite() {
                v.clamp(0.0, 1.0) as f32
            } else {
                0.0
            })
        });
        node.kind.replace(Kind::Leaf(Rc::new(move || {
            widget::progress_bar(0.0..=1.0, current.get()).into()
        })));
    }
    pub fn keep(&self, subscription: Subscription) {
        if self.0.closed.get() {
            return;
        }
        let target = self
            .0
            .template_scopes
            .borrow()
            .last()
            .and_then(Weak::upgrade);
        if let Some(target) = target {
            if target.mounted.get() {
                target.subscriptions.borrow_mut().push(subscription);
            }
        } else {
            self.0.subscriptions.borrow_mut().push(subscription);
        }
    }
    fn in_template(&self, target: &Rc<Node>, callback: impl FnOnce()) {
        struct Guard(Weak<Inner>);
        impl Drop for Guard {
            fn drop(&mut self) {
                if let Some(inner) = self.0.upgrade() {
                    inner.template_scopes.borrow_mut().pop();
                }
            }
        }
        self.0
            .template_scopes
            .borrow_mut()
            .push(Rc::downgrade(target));
        let _guard = Guard(Rc::downgrade(&self.0));
        callback();
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
    /// Replace the single app handler. Capture Store or WeakView, not a strong
    /// View. The handler decides when to force-close after confirmation/drain.
    pub fn on_close_request(&self, callback: Rc<dyn Fn()>) {
        if !self.0.closed.get() {
            drop(self.0.on_close_request.replace(Some(callback)));
        }
    }
    /// Call the app handler, or close by default. Synchronous reentrant requests
    /// are ignored, not queued; later requests may retry. Closed Views do nothing.
    pub fn request_close(&self) {
        if self.0.closed.get() || self.0.requesting_close.replace(true) {
            return;
        }
        let callback = self.0.on_close_request.borrow().clone();
        let result = catch_unwind(AssertUnwindSafe(|| {
            if let Some(callback) = callback {
                callback();
            } else {
                self.close();
            }
        }));
        self.0.requesting_close.set(false);
        if let Err(panic) = result {
            self.0.report(panic_error(panic));
        }
    }
    pub fn request_quit(&self) {
        if self.0.closed.get() {
            return;
        }
        let callback = self.0.quit_request.borrow().clone();
        let Some(callback) = callback else {
            self.request_close();
            return;
        };
        if self.0.requesting_close.replace(true) {
            return;
        }
        let result = catch_unwind(AssertUnwindSafe(|| callback()));
        self.0.requesting_close.set(false);
        if let Err(panic) = result {
            self.0.report(panic_error(panic));
        }
    }
    /// Explicit, idempotent force teardown. Never dispatches a close request.
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
        let target = match &message {
            Message::Edit(id, _)
            | Message::Click(id)
            | Message::Scroll(id, _)
            | Message::Custom(id)
            | Message::Toggle(id, _)
            | Message::Slide(id, _)
            | Message::Select(id, _)
            | Message::Open(id, _) => Some(id.as_str()),
            _ => None,
        };
        if let Some(dialog) = self.active_dialog()
            && (matches!(message, Message::Menu(_))
                || target.is_some_and(|target| !self.in_dialog(target, &dialog)))
        {
            return Ok(());
        }
        match message {
            Message::Ignored => Ok(()),
            Message::Toggle(id, value) => self.control_update(&id, controls::Input::Toggle(value)),
            Message::Slide(id, value) => self.control_update(&id, controls::Input::Slide(value)),
            Message::Select(id, value) => self.control_update(&id, controls::Input::Select(value)),
            Message::Open(id, value) => self.control_update(&id, controls::Input::Open(value)),
            Message::Edit(id, value) => {
                let node = self
                    .find(&id)
                    .ok_or_else(|| format!("missing entry: {id}"))?;
                let handler = node.handler.borrow().clone();
                if let Some(Handler::Entry(binding)) = handler {
                    if !binding.set(value) {
                        return Err(format!("read-only, invalid, or closed entry: {id}"));
                    }
                    if let Some(text) = node.text.borrow().as_ref() {
                        text.replace(binding.get());
                    }
                    self.0.refresh();
                    Ok(())
                } else {
                    Err(format!("not an entry: {id}"))
                }
            }
            Message::Click(id) => {
                let handler = self.find(&id).and_then(|n| n.handler.borrow().clone());
                if let Some(Handler::Toggle(binding)) = &handler {
                    return self.control_update(&id, controls::Input::Toggle(!binding.get()));
                }
                if let Some(Handler::Radio(binding, value)) = &handler {
                    binding
                        .try_set(value.clone())
                        .map_err(|error| error.to_string())?;
                    self.0.refresh();
                    return Ok(());
                }
                if let Some(Handler::Button(command)) = handler {
                    if command.invoke() {
                        Ok(())
                    } else {
                        Err(format!("disabled or closed command: {id}"))
                    }
                } else {
                    Err(format!("not a button: {id}"))
                }
            }
            Message::Menu(id) => self.activate_menu(&id),
            Message::Scroll(id, offset) => {
                let host = self.find(&id).and_then(|n| match &*n.kind.borrow() {
                    Kind::Items(h) => Some(h.clone()),
                    _ => None,
                });
                if let Some(host) = host {
                    host.scroll(offset);
                    Ok(())
                } else {
                    Err(format!("not a list: {id}"))
                }
            }
            Message::Custom(id) => {
                let callback = self.find(&id).and_then(|n| n.custom.borrow().clone());
                if let Some(callback) = callback {
                    callback();
                    Ok(())
                } else {
                    Err(format!("missing primitive callback: {id}"))
                }
            }
        }
    }
    pub fn edit(&self, id: &str, value: &str) -> Result<(), String> {
        if !self.find(id).is_some_and(|node| node.interactive()) {
            return Err("hidden, disabled or missing control".into());
        }
        self.dispatch(Message::Edit(id.into(), value.into()))
    }
    pub fn click(&self, id: &str) -> Result<(), String> {
        if !self.find(id).is_some_and(|node| node.interactive()) {
            return Err("hidden, disabled or missing control".into());
        }
        self.dispatch(Message::Click(id.into()))
    }
    pub fn text(&self, id: &str) -> Option<String> {
        self.find(id)
            .and_then(|n| n.text.borrow().as_ref().map(|v| v.borrow().clone()))
    }
    pub fn title(&self) -> String {
        self.0.title.borrow().clone()
    }
    pub fn diagnostics(&self) -> Vec<String> {
        self.0.diagnostics.borrow().clone()
    }
    pub fn error(&self) -> Option<String> {
        self.0.error.borrow().clone()
    }
    pub fn report_error(&self, error: String) {
        if !self.0.closed.get() {
            self.0.report(error);
        }
    }
    pub fn mount_identity(&self, id: &str) -> Option<usize> {
        self.find(id).map(|n| n.identity)
    }
    pub fn frame_count(&self) -> usize {
        self.0.frames.get()
    }
    pub fn subscription_count(&self) -> usize {
        self.0.subscriptions.borrow().len()
            + self
                .0
                .nodes
                .borrow()
                .values()
                .filter_map(Weak::upgrade)
                .map(|n| n.subscriptions.borrow().len())
                .sum::<usize>()
    }
    pub fn input_capabilities() -> InputCapabilities {
        InputCapabilities {
            system_clipboard: true,
            ime_commit: true,
            ime_preedit: false,
            physical_keys: false,
            os_accessibility: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputCapabilities {
    /// Connection errors are reported in diagnostics at window creation.
    pub system_clipboard: bool,
    pub ime_commit: bool,
    pub ime_preedit: bool,
    pub physical_keys: bool,
    pub os_accessibility: bool,
}

trait ItemHost {
    fn element(&self, node: &Node) -> Element;
    fn keys(&self) -> Vec<String>;
    fn viewport(&self, start: usize);
    fn scroll(&self, offset: f32);
}
struct List<T: Clone + 'static> {
    owner: Weak<Inner>,
    node: Weak<Node>,
    source: RefCell<Vec<Keyed<T>>>,
    viewport: Binding<Viewport>,
    current: Cell<Viewport>,
    viewport_ready: Cell<bool>,
    template: Template<T>,
    mounted: RefCell<Vec<(Keyed<T>, Rc<Node>)>>,
    busy: Cell<bool>,
    dirty: Cell<bool>,
    overscan: Cell<bool>,
}
pub fn virtual_range(len: usize, viewport: Viewport) -> std::ops::Range<usize> {
    viewport.range(len).unwrap_or(0..0)
}
fn row_height(viewport: Viewport) -> f32 {
    viewport.row_height
}
// Keyed's public fields can bypass its constructor. Keep this boundary check local.
fn valid_item_key(key: &str) -> bool {
    !key.is_empty() && !key.contains('/')
}
impl<T: Clone + 'static> List<T> {
    fn range(&self, len: usize) -> std::ops::Range<usize> {
        if !self.viewport_ready.get() {
            return 0..0;
        }
        let mut range = virtual_range(len, self.current.get());
        if self.overscan.get() {
            range.end = range.end.saturating_add(1).min(len);
        }
        range
    }
    fn reconcile(&self) {
        if self.busy.replace(true) {
            self.dirty.set(true);
            return;
        }
        struct Guard<'a>(&'a Cell<bool>);
        impl Drop for Guard<'_> {
            fn drop(&mut self) {
                self.0.set(false);
            }
        }
        let _guard = Guard(&self.busy);
        for _ in 0..32 {
            self.dirty.set(false);
            self.reconcile_once();
            if !self.dirty.get() {
                return;
            }
        }
        if let Some(inner) = self.owner.upgrade() {
            inner.report("template caused unbounded reentrant list updates".into());
        }
    }
    fn reconcile_once(&self) {
        let (Some(inner), Some(node)) = (self.owner.upgrade(), self.node.upgrade()) else {
            return;
        };
        if inner.closed.get() || !node.mounted.get() {
            return;
        }
        if self.viewport_ready.get() {
            let mut viewport = self.current.get();
            let start = virtual_range(self.source.borrow().len(), viewport).start;
            if viewport.start != start {
                viewport.start = start;
                self.current.set(viewport);
                let _ = self.viewport.set(viewport);
            }
        }
        let view = View(inner);
        let wanted = {
            let values = self.source.borrow();
            values[self.range(values.len())].to_vec()
        };
        let old = self.mounted.take();
        let mut retained = Vec::new();
        // Evict before building: a changed Store at the same key must not retain old bindings.
        for (item, mount) in old {
            if wanted.iter().any(|v| v == &item) {
                retained.push((item, mount));
            } else {
                mount.retire();
            }
        }
        node.children.borrow_mut().clear();
        let mut next = Vec::new();
        for item in &wanted {
            let mount = if let Some(index) = retained.iter().position(|(old, _)| old == item) {
                let (_, mount) = retained.remove(index);
                node.children.borrow_mut().push(mount.clone());
                mount
            } else {
                let scope = format!("{}/{}", node.id, item.key);
                let mount = view.add_scoped(
                    &Parent(node.clone()),
                    &scope,
                    Kind::Panel(Layout::Stack {
                        horizontal: false,
                        gap: 0.0,
                    }),
                    true,
                );
                view.in_template(&mount, || {
                    (self.template)(&view, &Parent(mount.clone()), &scope, item.model.clone())
                });
                mount
            };
            next.push((item.clone(), mount));
        }
        self.mounted.replace(next);
        view.0.refresh();
    }
}
impl<T: Clone + 'static> ItemHost for List<T> {
    fn element(&self, node: &Node) -> Element {
        if !self.viewport_ready.get() {
            return widget::Space::new().height(0.0).into();
        }
        let viewport = self.current.get();
        let height = row_height(viewport);
        let range = self.range(self.source.borrow().len());
        let mut children = vec![(
            "__before".into(),
            Style::default(),
            widget::Space::new()
                .height(range.start as f32 * height)
                .into(),
        )];
        for (_, mount) in self.mounted.borrow().iter() {
            let mut style = mount.style.get();
            let row_height =
                (height - layout::finite(style.margin.top) - layout::finite(style.margin.bottom))
                    .max(0.0);
            style.placement.height = Some(row_height);
            children.push((
                mount.key(),
                style,
                widget::container(mount.element())
                    .width(Length::Fill)
                    .height(row_height)
                    .clip(true)
                    .into(),
            ));
        }
        children.push((
            "__after".into(),
            Style::default(),
            widget::Space::new()
                .height((self.source.borrow().len() - range.end) as f32 * height)
                .into(),
        ));
        let id = node.id.clone();
        widget::scrollable(layout::Panel::element(
            Layout::Stack {
                horizontal: false,
                gap: 0.0,
            },
            children,
        ))
        .id(iced::advanced::widget::Id::from(id.clone()))
        .on_scroll(move |vp| Message::Scroll(id.clone(), vp.absolute_offset().y))
        .height(viewport.count as f32 * height)
        .width(Length::Fill)
        .into()
    }
    fn keys(&self) -> Vec<String> {
        self.mounted
            .borrow()
            .iter()
            .map(|(i, _)| i.key.clone())
            .collect()
    }
    fn viewport(&self, start: usize) {
        if !self.viewport_ready.get() {
            return;
        }
        let mut vp = self.current.get();
        vp.start = start.min(self.source.borrow().len().saturating_sub(vp.count));
        let had_overscan = self.overscan.replace(false);
        if vp != self.current.get() {
            self.current.set(vp);
            let _ = self.viewport.set(vp);
            self.reconcile();
        } else if had_overscan {
            self.reconcile();
        }
        if let (Some(inner), Some(node)) = (self.owner.upgrade(), self.node.upgrade()) {
            let start = virtual_range(self.source.borrow().len(), vp).start;
            inner
                .scroll
                .borrow_mut()
                .insert(node.id.clone(), start as f32 * row_height(vp));
            inner.refresh();
        }
    }
    fn scroll(&self, offset: f32) {
        if !self.viewport_ready.get() {
            return;
        }
        let mut vp = self.current.get();
        let start = ((layout::finite(offset) / row_height(vp)).floor() as usize)
            .min(self.source.borrow().len().saturating_sub(vp.count));
        let fractional = layout::finite(offset) % row_height(vp) > 0.01;
        let was_fractional = self.overscan.replace(fractional);
        if vp.start == start && was_fractional == fractional {
            return;
        }
        vp.start = start;
        self.current.set(vp);
        let _ = self.viewport.set(vp);
        self.reconcile();
    }
}
impl View {
    pub fn items<T: Clone + 'static>(
        &self,
        parent: &Parent,
        id: &str,
        items: Binding<Vec<Keyed<T>>>,
        viewport: Binding<Viewport>,
        template: Template<T>,
    ) {
        let node = self.add(parent, id, Kind::Content);
        let host = Rc::new(List {
            owner: Rc::downgrade(&self.0),
            node: Rc::downgrade(&node),
            source: RefCell::new(Vec::new()),
            current: Cell::new(Viewport::default()),
            viewport_ready: Cell::new(false),
            viewport: viewport.clone(),
            template,
            mounted: RefCell::default(),
            busy: Cell::new(false),
            dirty: Cell::new(false),
            overscan: Cell::new(false),
        });
        node.kind.replace(Kind::Items(host.clone()));
        let out = host.clone();
        let weak = Rc::downgrade(&self.0);
        let key = node.id.clone();
        self.bind(&node, items, move |values| {
            if let Some(invalid) = values.iter().find(|i| !valid_item_key(&i.key)) {
                if let Some(inner) = weak.upgrade() {
                    inner.report(format!(
                        "list {key}: invalid key {:?}; snapshot rejected",
                        invalid.key
                    ));
                }
                return;
            }
            let mut keys = HashSet::new();
            if let Some(duplicate) = values.iter().find(|i| !keys.insert(i.key.clone())) {
                if let Some(inner) = weak.upgrade() {
                    inner.report(format!(
                        "list {key}: duplicate key {:?}; snapshot rejected",
                        duplicate.key
                    ));
                }
                return;
            }
            out.source.replace(values);
            out.reconcile();
        });
        let out = host.clone();
        self.bind(&node, viewport, move |vp| {
            if let Err(error) = vp.validate() {
                if let Some(inner) = out.owner.upgrade() {
                    inner.report(error.into());
                }
                return;
            }
            let initialized = out.viewport_ready.replace(true);
            if out.current.replace(vp) != vp || !initialized {
                out.overscan.set(false);
                out.reconcile();
                if let (Some(inner), Some(node)) = (out.owner.upgrade(), out.node.upgrade()) {
                    let start = virtual_range(out.source.borrow().len(), vp).start;
                    inner
                        .scroll
                        .borrow_mut()
                        .insert(node.id.clone(), start as f32 * row_height(vp));
                }
            }
        });
        host.viewport(host.current.get().start);
    }
    pub fn content<T: Clone + 'static>(
        &self,
        parent: &Parent,
        id: &str,
        item: Binding<Option<Keyed<T>>>,
        template: Template<T>,
    ) {
        let node = self.add(parent, id, Kind::Content);
        let target = Rc::downgrade(&node);
        let weak = Rc::downgrade(&self.0);
        let previous = RefCell::new(None::<Keyed<T>>);
        self.bind(&node, item, move |item| {
            if let Some(invalid) = item.as_ref().filter(|i| !valid_item_key(&i.key)) {
                if let (Some(node), Some(inner)) = (target.upgrade(), weak.upgrade()) {
                    inner.report(format!(
                        "content {}: invalid key {:?}; snapshot rejected",
                        node.id, invalid.key
                    ));
                }
                return;
            }
            if *previous.borrow() == item {
                return;
            }
            previous.replace(item.clone());
            if let (Some(node), Some(inner)) = (target.upgrade(), weak.upgrade()) {
                for child in node.children.take() {
                    child.retire();
                }
                if let Some(item) = item {
                    let view = View(inner);
                    let scope = format!("{}/{}", node.id, item.key);
                    let parent = Parent(view.add_scoped(
                        &Parent(node),
                        &scope,
                        Kind::Panel(Layout::Stack {
                            horizontal: false,
                            gap: 0.0,
                        }),
                        true,
                    ));
                    view.in_template(&parent.0, || (template)(&view, &parent, &scope, item.model));
                }
            }
        });
    }
    pub fn item_keys(&self, id: &str) -> Vec<String> {
        self.find(id)
            .map(|n| match &*n.kind.borrow() {
                Kind::Items(h) => h.keys(),
                _ => Vec::new(),
            })
            .unwrap_or_default()
    }
    pub fn mounted_count(&self, id: &str) -> usize {
        self.item_keys(id).len()
    }
    pub fn viewport(&self, id: &str, start: usize) {
        let host = self.find(id).and_then(|n| match &*n.kind.borrow() {
            Kind::Items(h) => Some(h.clone()),
            _ => None,
        });
        if let Some(host) = host {
            host.viewport(start);
        } else {
            self.0
                .report(format!("viewport target is not a mounted list: {id}"));
        }
    }
}

fn panic_error(panic: Box<dyn std::any::Any + Send>) -> String {
    format!(
        "Tao callback panicked: {}",
        panic
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| panic.downcast_ref::<&str>().copied())
            .unwrap_or("non-string panic")
    )
}
/// Owns the native loop and UI-local scheduler. Main thread only; no GTK4 in this process.
pub fn run(make: impl FnOnce(Rc<dyn Executor>) -> View + 'static) -> Result<(), String> {
    run_host(move |host| {
        host.present(&make(panechord_runtime::UiHost::executor(host.as_ref())));
        Ok(())
    })
}
pub struct TaoHost {
    executor: Rc<scheduler::Scheduler>,
    views: RefCell<Vec<(usize, View)>>,
    session: RefCell<Option<panechord_runtime::Session>>,
    next: Cell<usize>,
    notify: Rc<dyn Fn(usize)>,
}
impl panechord_runtime::UiHost for TaoHost {
    fn executor(&self) -> Rc<dyn Executor> {
        self.executor.clone()
    }
    fn attach(&self, session: panechord_runtime::Session) -> Result<(), String> {
        if session.shutdown().phase() == panechord_runtime::ShutdownPhase::Closed {
            return Err("cannot attach a closed Session".into());
        }
        if let Some(attached) = self.session.borrow().as_ref() {
            return if attached.ptr_eq(&session) {
                Ok(())
            } else {
                Err("TaoHost supports one Session; share it across all windows".into())
            };
        }
        self.session.replace(Some(session));
        Ok(())
    }
}
impl TaoHost {
    pub fn present(&self, view: &View) {
        if view.0.closed.get() {
            return;
        }
        if !self
            .views
            .borrow()
            .iter()
            .any(|(_, existing)| Rc::ptr_eq(&existing.0, &view.0))
        {
            let id = self.next.get();
            self.next
                .set(id.checked_add(1).expect("window ID exhausted"));
            let notify = self.notify.clone();
            view.0.invalidate.replace(Some(Rc::new(move || notify(id))));
            self.views.borrow_mut().push((id, view.clone()));
        }
        view.present();
    }
    pub fn register_view(
        &self,
        session: &panechord_runtime::Session,
        id: &str,
        view: &View,
        hooks: panechord_runtime::WindowHooks,
    ) -> Result<(), String> {
        if view.0.closed.get() {
            return Err("cannot register a closed View".into());
        }
        panechord_runtime::UiHost::attach(self, session.clone())?;
        let token = session.register_window(id, hooks)?;
        view.keep(token);
        let shutdown = session.shutdown();
        let executor = self.executor.clone();
        view.0.quit_request.replace(Some(Rc::new(move || {
            let shutdown = shutdown.clone();
            executor.spawn(Box::pin(async move {
                shutdown
                    .request_shutdown(std::time::Duration::from_secs(5))
                    .await;
            }));
        })));
        self.present(view);
        Ok(())
    }
}
impl Drop for TaoHost {
    fn drop(&mut self) {
        if let Some(session) = self.session.get_mut().take() {
            session.shutdown().close_now();
        }
        for (_, view) in self.views.get_mut().drain(..) {
            view.close();
        }
        self.executor.close();
    }
}
pub fn run_host(
    make: impl FnOnce(Rc<TaoHost>) -> Result<(), String> + 'static,
) -> Result<(), String> {
    driver::run(make)
}
/// Rejects external reactor integration; use `run` on the main thread instead.
pub async fn run_async(
    _make: impl FnOnce(Rc<dyn Executor>) -> View + 'static,
) -> Result<(), String> {
    Err("external reactor host is unsupported; use panechord_tao::run on the main thread".into())
}
// Attached-host execution is intentionally rejected rather than starting a nested loop.

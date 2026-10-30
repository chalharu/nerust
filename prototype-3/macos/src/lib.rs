//! Main-thread AppKit host for the third prototype's practical screens.
//! Native controls own editing/marked text; Rust owns bindings and scoped tokens.
#![cfg(target_os = "macos")]

use futures::future::LocalBoxFuture;
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet, VecDeque},
    ffi::c_void,
    panic::{AssertUnwindSafe, catch_unwind},
    rc::{Rc, Weak},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll, Wake, Waker},
    time::{Duration, Instant},
};
use wakui2_runtime::{
    Binding, Command, Executor, Keyed, Layout, MenuItem, Shortcut, Store, Style, Subscription,
    Viewport,
};

type Callback = unsafe extern "C" fn(*mut c_void, *const u8, usize, *const u8, usize);

unsafe extern "C" {
    fn w3_create(callback: Callback, context: *mut c_void) -> *mut c_void;
    fn w3_destroy(host: *mut c_void);
    fn w3_present(host: *mut c_void);
    fn w3_close(host: *mut c_void);
    fn w3_focused(host: *mut c_void) -> i32;
    fn w3_render(host: *mut c_void);
    fn w3_pump(milliseconds: i32);
    fn w3_add(
        host: *mut c_void,
        parent: *const u8,
        parent_len: usize,
        id: *const u8,
        id_len: usize,
        kind: i32,
        number: f64,
    );
    fn w3_set(
        host: *mut c_void,
        id: *const u8,
        id_len: usize,
        text: *const u8,
        text_len: usize,
        property: i32,
    );
    fn w3_text(
        host: *mut c_void,
        id: *const u8,
        id_len: usize,
        output: *mut u8,
        capacity: usize,
    ) -> isize;
    fn w3_action(
        host: *mut c_void,
        id: *const u8,
        id_len: usize,
        text: *const u8,
        text_len: usize,
        kind: i32,
    ) -> i32;
    fn w3_style(
        host: *mut c_void,
        id: *const u8,
        id_len: usize,
        top: f64,
        right: f64,
        bottom: f64,
        left: f64,
        width: f64,
        height: f64,
    );
    fn w3_remove(host: *mut c_void, id: *const u8, id_len: usize);
    fn w3_rows(
        host: *mut c_void,
        id: *const u8,
        id_len: usize,
        total: usize,
        start: usize,
        count: usize,
        height: f64,
    );
    fn w3_menu(host: *mut c_void, id: *const u8, id_len: usize, json: *const u8, json_len: usize);
    fn w3_input_probe(host: *mut c_void, id: *const u8, id_len: usize) -> i32;
    fn w3_post_key(host: *mut c_void, id: *const u8, id_len: usize) -> i32;
    fn w3_shortcut(host: *mut c_void, value: *const u8, value_len: usize) -> i32;
    fn w3_capture(host: *mut c_void, path: *const u8, path_len: usize, width: f64) -> i32;
}

#[derive(Clone)]
pub struct Parent(String);

type Handler = Rc<dyn Fn(String)>;
type Template<T> = Rc<dyn Fn(&View, &Parent, &str, Store<T>)>;
type Refresh = Rc<dyn Fn(&View)>;

#[derive(Default)]
struct Scope {
    tokens: Vec<Subscription>,
}

struct List {
    keys: RefCell<Vec<String>>,
    mounted: RefCell<Vec<String>>,
    error: RefCell<Option<String>>,
    viewport: Binding<Viewport>,
}

struct Inner {
    native: Cell<*mut c_void>,
    scopes: RefCell<HashMap<String, Scope>>,
    handlers: RefCell<HashMap<String, Handler>>,
    nodes: RefCell<HashSet<String>>,
    lists: RefCell<HashMap<String, Rc<List>>>,
    close_handlers: RefCell<Vec<Rc<dyn Fn()>>>,
    close_request: RefCell<Option<Rc<dyn Fn()>>>,
    requesting: Cell<bool>,
    closed: Cell<bool>,
    error: RefCell<Option<String>>,
}

impl Inner {
    fn shutdown(&self) {
        if self.closed.replace(true) {
            return;
        }
        drop(self.close_request.take());
        drop(self.scopes.take());
        drop(self.handlers.take());
        drop(self.lists.take());
        unsafe {
            w3_close(self.native.get());
        }
        for callback in self.close_handlers.take() {
            if catch_unwind(AssertUnwindSafe(|| callback())).is_err() {
                self.error.replace(Some("close callback panicked".into()));
            }
        }
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.shutdown();
        unsafe {
            w3_destroy(self.native.get());
        }
    }
}

#[derive(Clone)]
pub struct View(Rc<Inner>);

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

unsafe extern "C" fn dispatch(
    context: *mut c_void,
    id: *const u8,
    id_len: usize,
    text: *const u8,
    text_len: usize,
) {
    let inner = unsafe { &*context.cast::<Inner>() };
    if inner.closed.get() {
        return;
    }
    let result = catch_unwind(AssertUnwindSafe(|| {
        let id = String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(id, id_len) });
        let text = if text_len == 0 {
            String::new()
        } else {
            String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(text, text_len) })
                .into_owned()
        };
        if id == "__close" {
            if inner.requesting.replace(true) {
                return;
            }
            let callback = inner.close_request.borrow().clone();
            if let Some(callback) = callback {
                callback();
            } else {
                inner.shutdown();
            }
            inner.requesting.set(false);
        } else {
            let handler = inner.handlers.borrow().get(id.as_ref()).cloned();
            if let Some(handler) = handler {
                handler(text);
            }
        }
    }));
    if result.is_err() {
        inner
            .error
            .replace(Some("native event callback panicked".into()));
        inner.shutdown();
    }
}

fn within(id: &str, scope: &str) -> bool {
    id == scope
        || id
            .strip_prefix(scope)
            .is_some_and(|tail| tail.starts_with('/'))
}

impl View {
    pub fn new(title: Binding<String>) -> Self {
        let inner = Rc::new(Inner {
            native: Cell::new(std::ptr::null_mut()),
            scopes: RefCell::new(HashMap::new()),
            handlers: RefCell::new(HashMap::new()),
            nodes: RefCell::new(HashSet::from(["__root".into()])),
            lists: RefCell::new(HashMap::new()),
            close_handlers: RefCell::new(Vec::new()),
            close_request: RefCell::new(None),
            requesting: Cell::new(false),
            closed: Cell::new(false),
            error: RefCell::new(None),
        });
        inner
            .native
            .set(unsafe { w3_create(dispatch, Rc::as_ptr(&inner).cast_mut().cast()) });
        let view = Self(inner);
        view.bind("__root", title, |view, value| {
            view.set("__title", &value, 0)
        });
        view
    }

    pub fn root(&self) -> Parent {
        Parent("__root".into())
    }
    pub fn downgrade(&self) -> WeakView {
        WeakView(Rc::downgrade(&self.0))
    }
    pub fn keep(&self, token: Subscription) {
        self.keep_at("__root", token);
    }
    fn keep_at(&self, id: &str, token: Subscription) {
        if !self.0.closed.get() {
            self.0
                .scopes
                .borrow_mut()
                .entry(id.into())
                .or_default()
                .tokens
                .push(token);
        }
    }
    fn bind<T: Clone + 'static>(
        &self,
        id: &str,
        binding: Binding<T>,
        callback: impl Fn(&View, T) + 'static,
    ) {
        let weak = self.downgrade();
        let token = binding.subscribe(Rc::new(move |value| {
            if let Some(view) = weak.upgrade() {
                callback(&view, value);
            }
        }));
        self.keep_at(id, token);
    }
    fn set(&self, id: &str, text: &str, property: i32) {
        if !self.0.closed.get() {
            unsafe {
                w3_set(
                    self.0.native.get(),
                    id.as_ptr(),
                    id.len(),
                    text.as_ptr(),
                    text.len(),
                    property,
                );
            }
        }
    }
    fn add(&self, parent: &Parent, id: &str, kind: i32, number: f64) {
        if self.0.closed.get() {
            return;
        }
        if !self.0.nodes.borrow_mut().insert(id.into()) {
            self.0
                .error
                .replace(Some(format!("duplicate native node: {id}")));
            return;
        }
        unsafe {
            w3_add(
                self.0.native.get(),
                parent.0.as_ptr(),
                parent.0.len(),
                id.as_ptr(),
                id.len(),
                kind,
                number,
            );
        }
    }
    fn remove(&self, prefix: &str) {
        let ids: Vec<_> = self
            .0
            .nodes
            .borrow()
            .iter()
            .filter(|id| within(id, prefix))
            .cloned()
            .collect();
        let mut removed = Vec::new();
        for id in ids {
            self.0.nodes.borrow_mut().remove(&id);
            removed.push(self.0.scopes.borrow_mut().remove(&id));
            let handler = self.0.handlers.borrow_mut().remove(&id);
            drop(handler);
        }
        drop(removed);
        unsafe {
            w3_remove(self.0.native.get(), prefix.as_ptr(), prefix.len());
        }
    }
    pub fn layout(&self, parent: &Parent, id: &str, layout: Layout) -> Parent {
        if let Layout::Stack { horizontal, gap } = layout {
            self.add(parent, id, i32::from(horizontal), f64::from(gap));
        } else {
            self.0
                .error
                .replace(Some("AppKit currently supports Stack layouts only".into()));
        }
        Parent(id.into())
    }
    pub fn style(&self, id: &str, style: Style) {
        let padding = style.padding;
        let margin = style.margin;
        let placement = style.placement;
        unsafe {
            w3_style(
                self.0.native.get(),
                id.as_ptr(),
                id.len(),
                f64::from(padding.top + margin.top),
                f64::from(padding.right + margin.right),
                f64::from(padding.bottom + margin.bottom),
                f64::from(padding.left + margin.left),
                f64::from(placement.width.unwrap_or(0.0)),
                f64::from(placement.height.unwrap_or(0.0)),
            );
        }
    }
    pub fn label(&self, parent: &Parent, id: &str, text: Binding<String>) {
        self.add(parent, id, 2, 0.0);
        let key = id.to_owned();
        self.bind(id, text, move |view, value| view.set(&key, &value, 0));
    }
    pub fn entry(&self, parent: &Parent, id: &str, text: Binding<String>) {
        self.add(parent, id, 3, if text.is_writable() { 1.0 } else { 0.0 });
        let key = id.to_owned();
        let weak = self.downgrade();
        let writer = text.clone();
        self.0.handlers.borrow_mut().insert(
            id.into(),
            Rc::new(move |value| {
                if !writer.set(value)
                    && let Some(view) = weak.upgrade()
                {
                    view.set(&key, &writer.get(), 0);
                }
            }),
        );
        let key = id.to_owned();
        self.bind(id, text, move |view, value| view.set(&key, &value, 0));
    }
    pub fn button(&self, parent: &Parent, id: &str, text: Binding<String>, command: Command) {
        self.add(parent, id, 4, 0.0);
        let key = id.to_owned();
        self.bind(id, text, move |view, value| view.set(&key, &value, 0));
        let key = id.to_owned();
        self.bind(id, command.enabled.clone(), move |view, value| {
            view.set(&key, if value { "1" } else { "0" }, 1)
        });
        self.0.handlers.borrow_mut().insert(
            id.into(),
            Rc::new(move |_| {
                command.invoke();
            }),
        );
    }

    pub fn items<T: Clone + 'static>(
        &self,
        parent: &Parent,
        id: &str,
        items: Binding<Vec<Keyed<T>>>,
        viewport: Binding<Viewport>,
        template: Template<T>,
    ) {
        self.add(parent, id, 5, 0.0);
        let list = Rc::new(List {
            keys: RefCell::new(Vec::new()),
            mounted: RefCell::new(Vec::new()),
            error: RefCell::new(None),
            viewport: viewport.clone(),
        });
        self.0.lists.borrow_mut().insert(id.into(), list.clone());
        let key = id.to_owned();
        let mounted = Rc::new(RefCell::new(Vec::<Keyed<T>>::new()));
        let source = items.clone();
        let range = viewport.clone();
        let refresh: Refresh = Rc::new(move |view| {
            let snapshot = source.get();
            let mut seen = HashSet::new();
            if snapshot.iter().any(|row| {
                !wakui2_runtime::valid_item_key(&row.key) || !seen.insert(row.key.clone())
            }) {
                list.error
                    .replace(Some("invalid or duplicate item key".into()));
                return;
            }
            let viewport = range.get();
            let start = match viewport.range(snapshot.len()) {
                Ok(range) => range.start,
                Err(error) => {
                    list.error.replace(Some(error.into()));
                    return;
                }
            };
            let visible: Vec<_> = snapshot
                .iter()
                .skip(start)
                .take(viewport.count)
                .cloned()
                .collect();
            let old = mounted.replace(visible.clone());
            for row in &old {
                if !visible.contains(row) {
                    view.remove(&format!("{key}/{}", row.key));
                }
            }
            let order_changed = old != visible;
            if order_changed {
                for row in &visible {
                    if old.contains(row) {
                        view.remove(&format!("{key}/{}", row.key));
                    }
                }
            }
            for row in &visible {
                if order_changed || !old.contains(row) {
                    let scope = format!("{key}/{}", row.key);
                    let row_parent = view.layout(
                        &Parent(key.clone()),
                        &scope,
                        Layout::Stack {
                            horizontal: false,
                            gap: 0.0,
                        },
                    );
                    let mut style = Style::default();
                    style.placement.height = Some(viewport.row_height);
                    view.style(&scope, style);
                    template(view, &row_parent, &scope, row.model.clone());
                }
            }
            list.keys
                .replace(snapshot.iter().map(|row| row.key.clone()).collect());
            list.mounted
                .replace(visible.iter().map(|row| row.key.clone()).collect());
            list.error.take();
            unsafe {
                w3_rows(
                    view.0.native.get(),
                    key.as_ptr(),
                    key.len(),
                    snapshot.len(),
                    start,
                    viewport.count,
                    f64::from(viewport.row_height),
                );
            }
            let mut style = Style::default();
            style.placement.height = Some(viewport.count as f32 * viewport.row_height);
            view.style(&key, style);
        });
        let update = refresh.clone();
        self.bind(id, items, move |view, _| update(view));
        self.bind(id, viewport.clone(), move |view, _| refresh(view));
        self.0.handlers.borrow_mut().insert(
            format!("{id}@viewport"),
            Rc::new(move |value| {
                if let Ok(start) = value.parse() {
                    let mut next = viewport.get();
                    next.start = start;
                    viewport.set(next);
                }
            }),
        );
    }
    pub fn viewport(&self, id: &str, start: usize) {
        let list = self.0.lists.borrow().get(id).cloned();
        if let Some(list) = list {
            let mut next = list.viewport.get();
            next.start = start;
            list.viewport.set(next);
        }
    }
    pub fn item_keys(&self, id: &str) -> Vec<String> {
        self.0
            .lists
            .borrow()
            .get(id)
            .map(|list| list.keys.borrow().clone())
            .unwrap_or_default()
    }
    pub fn mounted_count(&self, id: &str) -> usize {
        self.0
            .lists
            .borrow()
            .get(id)
            .map_or(0, |list| list.mounted.borrow().len())
    }
    pub fn items_error(&self, id: &str) -> Option<String> {
        self.0
            .lists
            .borrow()
            .get(id)
            .and_then(|list| list.error.borrow().clone())
    }
    pub fn menu(
        &self,
        id: &str,
        title: Binding<String>,
        items: Binding<Vec<MenuItem>>,
        dispatch: Rc<dyn Fn(String) -> bool>,
    ) {
        let key = id.to_owned();
        let source = items.clone();
        let heading = title.clone();
        let refresh: Refresh = Rc::new(move |view| {
            let prefix = format!("{key}/");
            let keys: Vec<_> = view
                .0
                .handlers
                .borrow()
                .keys()
                .filter(|id| id.starts_with(&prefix))
                .cloned()
                .collect();
            for id in keys {
                let old = view.0.handlers.borrow_mut().remove(&id);
                drop(old);
            }
            let mut seen = HashSet::new();
            let mut native = Vec::new();
            for item in source.get() {
                if !wakui2_runtime::valid_item_key(&item.id) || !seen.insert(item.id.clone()) {
                    view.0
                        .error
                        .replace(Some("invalid or duplicate menu item id".into()));
                    return;
                }
                let shortcut = match item.shortcut.as_deref().map(Shortcut::parse).transpose() {
                    Ok(shortcut) => shortcut
                        .map(|shortcut| shortcut.to_muda())
                        .unwrap_or_default(),
                    Err(error) => {
                        view.0.error.replace(Some(error.to_string()));
                        return;
                    }
                };
                let argument = item.argument.clone();
                let invoke = dispatch.clone();
                let enabled = item.enabled;
                native.push(serde_json::json!({"id":item.id,"label":item.label,"shortcut":shortcut,"role":format!("{:?}", item.role),"enabled":enabled}));
                view.0.handlers.borrow_mut().insert(
                    format!("{key}/{}", item.id),
                    Rc::new(move |_| {
                        if enabled {
                            invoke(argument.clone());
                        }
                    }),
                );
            }
            let json = serde_json::json!({"title":heading.get(),"items":native}).to_string();
            unsafe {
                w3_menu(
                    view.0.native.get(),
                    key.as_ptr(),
                    key.len(),
                    json.as_ptr(),
                    json.len(),
                );
            }
        });
        let update = refresh.clone();
        self.bind(id, title, move |view, _| update(view));
        self.bind(id, items, move |view, _| refresh(view));
    }

    pub fn on_close(&self, callback: Rc<dyn Fn()>) {
        if self.0.closed.get() {
            callback();
        } else {
            self.0.close_handlers.borrow_mut().push(callback);
        }
    }
    pub fn on_close_request(&self, callback: Rc<dyn Fn()>) {
        self.0.close_request.replace(Some(callback));
    }
    pub fn request_close(&self) {
        if self.0.closed.get() || self.0.requesting.replace(true) {
            return;
        }
        let callback = self.0.close_request.borrow().clone();
        if let Some(callback) = callback {
            callback();
        } else {
            self.close();
        }
        self.0.requesting.set(false);
    }
    pub fn close(&self) {
        self.0.shutdown();
    }
    pub fn present(&self) {
        if !self.0.closed.get() {
            unsafe {
                w3_present(self.0.native.get());
            }
        }
    }
    pub fn title(&self) -> String {
        self.text("__title").unwrap_or_default()
    }
    pub fn is_key_window(&self) -> bool {
        !self.0.closed.get() && unsafe { w3_focused(self.0.native.get()) } == 1
    }
    pub fn text(&self, id: &str) -> Option<String> {
        if self.0.closed.get() {
            return None;
        }
        let size = unsafe {
            w3_text(
                self.0.native.get(),
                id.as_ptr(),
                id.len(),
                std::ptr::null_mut(),
                0,
            )
        };
        if size < 0 {
            return None;
        }
        let mut buffer = vec![0; size as usize];
        unsafe {
            w3_text(
                self.0.native.get(),
                id.as_ptr(),
                id.len(),
                buffer.as_mut_ptr(),
                buffer.len(),
            );
        }
        String::from_utf8(buffer).ok()
    }
    fn action(&self, id: &str, value: &str, kind: i32) -> Result<(), String> {
        if self.0.closed.get() {
            return Err("view is closed".into());
        }
        if unsafe {
            w3_action(
                self.0.native.get(),
                id.as_ptr(),
                id.len(),
                value.as_ptr(),
                value.len(),
                kind,
            )
        } == 1
        {
            Ok(())
        } else {
            Err(format!(
                "native control unavailable, readonly or disabled: {id}"
            ))
        }
    }
    pub fn edit(&self, id: &str, value: &str) -> Result<(), String> {
        self.action(id, value, 0)
    }
    pub fn click(&self, id: &str) -> Result<(), String> {
        self.action(id, "", 1)
    }
    pub fn activate_menu(&self, id: &str) -> Result<(), String> {
        self.action(id, "", 2)
    }
    /// AppKit field-editor injection, not a physical keyboard or external IME test.
    pub fn probe_input(&self, id: &str) -> Result<(), String> {
        if self.0.closed.get() {
            return Err("view is closed".into());
        }
        let code = unsafe { w3_input_probe(self.0.native.get(), id.as_ptr(), id.len()) };
        if code == 1 {
            Ok(())
        } else {
            Err(format!(
                "AppKit field-editor probe failed at stage {}",
                -code
            ))
        }
    }
    /// Posts a synthetic AppKit key event, without Accessibility permission.
    pub fn post_test_key(&self, id: &str) -> bool {
        !self.0.closed.get()
            && unsafe { w3_post_key(self.0.native.get(), id.as_ptr(), id.len()) } == 1
    }
    /// AppKit drawing CPU time, not compositor presentation or a frame fence.
    pub fn render(&self) -> Duration {
        let started = Instant::now();
        if !self.0.closed.get() {
            unsafe {
                w3_render(self.0.native.get());
            }
        }
        started.elapsed()
    }
    pub fn perform_shortcut(&self, shortcut: &str) -> Result<bool, String> {
        if self.0.closed.get() {
            return Err("view is closed".into());
        }
        let shortcut = Shortcut::parse(shortcut)
            .map_err(|error| error.to_string())?
            .to_muda();
        Ok(unsafe { w3_shortcut(self.0.native.get(), shortcut.as_ptr(), shortcut.len()) } == 1)
    }
    pub fn capture(&self, path: &std::path::Path, width: f64) -> Result<(), String> {
        let path = path.to_str().ok_or("capture path must be UTF-8")?;
        if !self.0.closed.get()
            && unsafe { w3_capture(self.0.native.get(), path.as_ptr(), path.len(), width) } == 1
        {
            Ok(())
        } else {
            Err("native capture failed".into())
        }
    }
}

struct Ready(AtomicBool);
impl Wake for Ready {
    fn wake(self: Arc<Self>) {
        self.0.store(true, Ordering::Release);
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.0.store(true, Ordering::Release);
    }
}
struct Task {
    future: LocalBoxFuture<'static, ()>,
    ready: Arc<Ready>,
}
#[derive(Default)]
struct MacExecutor {
    queue: RefCell<VecDeque<Task>>,
}
impl Executor for MacExecutor {
    fn spawn(&self, future: LocalBoxFuture<'static, ()>) {
        self.queue.borrow_mut().push_back(Task {
            future,
            ready: Arc::new(Ready(AtomicBool::new(true))),
        });
    }
}
impl MacExecutor {
    fn has_ready(&self) -> bool {
        self.queue
            .borrow()
            .iter()
            .any(|task| task.ready.0.load(Ordering::Acquire))
    }
    fn turn(&self) -> usize {
        let count = self.queue.borrow().len().min(32);
        let mut polled = 0;
        for _ in 0..count {
            let task = self.queue.borrow_mut().pop_front();
            if let Some(mut task) = task {
                if task.ready.0.swap(false, Ordering::AcqRel) {
                    polled += 1;
                    let waker = Waker::from(task.ready.clone());
                    let mut context = Context::from_waker(&waker);
                    if task.future.as_mut().poll(&mut context) == Poll::Ready(()) {
                        continue;
                    }
                }
                self.queue.borrow_mut().push_back(task);
            }
        }
        polled
    }
}

pub fn run(make: impl FnOnce(Rc<dyn Executor>) -> View + 'static) -> Result<(), String> {
    let executor = Rc::new(MacExecutor::default());
    let view = make(executor.clone());
    let initial_error = view.0.error.borrow().clone();
    if let Some(error) = initial_error {
        view.close();
        return Err(error);
    }
    view.present();
    let deadline = std::env::var("WAKUI_MACOS_TIMEOUT_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok());
    let started = Instant::now();
    let mut previous = started;
    let mut gaps = Vec::new();
    let result = catch_unwind(AssertUnwindSafe(|| {
        while !view.0.closed.get() {
            if deadline.is_some_and(|seconds| started.elapsed() > Duration::from_secs(seconds)) {
                return Err("macOS host deadline exceeded".to_owned());
            }
            let now = Instant::now();
            if gaps.len() < 100_000 {
                gaps.push(now.duration_since(previous));
            }
            previous = now;
            unsafe {
                w3_pump(if executor.has_ready() { 0 } else { 8 });
            }
            if !view.0.closed.get() {
                executor.turn();
            }
            if let Some(error) = view.0.error.borrow().clone() {
                return Err(error);
            }
        }
        Ok(())
    }))
    .unwrap_or_else(|_| Err("macOS host future panicked".into()));
    view.close();
    drop(executor.queue.take());
    if std::env::var_os("WAKUI_MACOS_METRICS").is_some() && !gaps.is_empty() {
        gaps.sort_unstable();
        println!(
            "MACOS3 host turns={} gap_p50_us={} gap_p95_us={} gap_max_us={} polls_per_turn<=32 (CPU host-turn observation, not physical input latency)",
            gaps.len(),
            gaps[gaps.len() / 2].as_micros(),
            gaps[gaps.len() * 95 / 100].as_micros(),
            gaps.last().unwrap().as_micros()
        );
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn executor_limits_polls_and_does_not_poll_spawned_work_in_same_turn() {
        let executor = Rc::new(MacExecutor::default());
        let count = Rc::new(Cell::new(0));
        for _ in 0..100 {
            let counter = count.clone();
            executor.spawn(Box::pin(async move {
                counter.set(counter.get() + 1);
            }));
        }
        assert_eq!(executor.turn(), 32);
        assert_eq!(count.get(), 32);
        assert_eq!(executor.turn(), 32);
        assert_eq!(executor.turn(), 32);
        assert_eq!(executor.turn(), 4);
        assert_eq!(count.get(), 100);
        let next = executor.clone();
        let counter = count.clone();
        executor.spawn(Box::pin(async move {
            next.spawn(Box::pin(async move {
                counter.set(101);
            }));
        }));
        assert_eq!(executor.turn(), 1);
        assert_eq!(count.get(), 100);
        assert_eq!(executor.turn(), 1);
        assert_eq!(count.get(), 101);
    }

    #[test]
    fn pending_future_is_not_repolled_until_woken() {
        let executor = MacExecutor::default();
        let captured = Rc::new(RefCell::new(None));
        let output = captured.clone();
        executor.spawn(Box::pin(futures::future::poll_fn(move |context| {
            output.replace(Some(context.waker().clone()));
            Poll::Pending
        })));
        assert_eq!(executor.turn(), 1);
        assert!(!executor.has_ready());
        assert_eq!(executor.turn(), 0);
        captured.borrow().as_ref().unwrap().wake_by_ref();
        assert!(executor.has_ready());
        assert_eq!(executor.turn(), 1);
    }
}

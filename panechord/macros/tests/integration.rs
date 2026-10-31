use futures::{executor::LocalPool, task::LocalSpawnExt};
use panechord_macros::{commands, owned_view, view};
use panechord_runtime::*;
use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

#[derive(Clone)]
struct Leaf {
    name: String,
}
#[derive(Clone)]
struct Row {
    name: String,
    error: String,
    pending: bool,
    progress: f64,
    children: Vec<Keyed<Leaf>>,
    selected: Option<Keyed<Leaf>>,
}
#[commands]
impl Row {
    #[command]
    fn rename(&mut self, value: String) {
        self.name = value;
    }
}
#[derive(Clone)]
struct Settings {
    title: String,
    name: String,
    argument: String,
    progress: f64,
    error: String,
    pending: bool,
    allow_save: bool,
    calls: Vec<String>,
    rows: Vec<Keyed<Row>>,
    selected: Option<Keyed<Row>>,
    viewport: Viewport,
    menu_items: Vec<MenuItem>,
}
#[commands]
impl Settings {
    #[command]
    fn add(&mut self, value: String) {
        self.calls.push(value);
    }
    #[command]
    fn reset(&mut self) {
        self.calls.push("reset".into());
    }
    #[command(can_execute = "can_save")]
    fn save(model: Store<Self>, argument: String) {
        // A static action can freely snapshot/update: it is outside a state borrow.
        assert!(model.snapshot().allow_save);
        let _ = model.update(move |s| s.calls.push(argument));
    }
    fn can_save(&self) -> bool {
        self.allow_save
    }
    #[command]
    fn choose(&mut self, arg: String) {
        self.calls.push(format!("menu:{arg}"));
    }
}
async fn validate_name(value: String) -> Result<(), String> {
    if value.is_empty() {
        Err("required".into())
    } else {
        Ok(())
    }
}
struct NumberText;
impl NumberText {
    fn forward(value: f64) -> String {
        value.to_string()
    }
    fn backward(value: String) -> Result<f64, String> {
        value.parse().map_err(|_| "number required".into())
    }
}
fn meter(view: &fake::View, parent: &fake::Parent, id: &str, value: Binding<f64>) {
    view.meter(parent, id, value);
}

// Backend-free construction probe. Template mounting is eager for the requested
// initial viewport; production recycling/scroll behavior belongs to backend tests.
mod fake {
    use super::*;
    type Template<T> = Rc<dyn Fn(&View, &Parent, &str, Store<T>)>;
    #[derive(Clone, Default)]
    pub struct Parent;
    #[derive(Default)]
    pub struct State {
        pub title: Option<Binding<String>>,
        pub texts: BTreeMap<String, Binding<String>>,
        pub commands: BTreeMap<String, Command>,
        pub values: BTreeMap<String, Binding<f64>>,
        pub viewports: BTreeMap<String, Binding<Viewport>>,
        pub layouts: BTreeMap<String, Layout>,
        pub styles: BTreeMap<String, Style>,
        pub menus: BTreeMap<String, Binding<Vec<MenuItem>>>,
        pub dispatch: BTreeMap<String, Rc<dyn Fn(String) -> bool>>,
        pub tokens: Vec<Subscription>,
        pub close: Option<Rc<dyn Fn()>>,
    }
    #[derive(Clone, Default)]
    pub struct View(pub Rc<RefCell<State>>);
    impl View {
        pub fn try_layout(
            &self,
            parent: &Parent,
            id: &str,
            value: Layout,
        ) -> Result<Parent, String> {
            Ok(self.layout(parent, id, value))
        }
        pub fn report_error(&self, error: String) {
            panic!("{error}");
        }
        pub fn new(title: Binding<String>) -> Self {
            let v = Self::default();
            v.0.borrow_mut().title = Some(title);
            v
        }
        pub fn root(&self) -> Parent {
            Parent
        }
        pub fn layout(&self, _: &Parent, id: &str, value: Layout) -> Parent {
            assert!(
                self.0
                    .borrow_mut()
                    .layouts
                    .insert(id.into(), value)
                    .is_none()
            );
            Parent
        }
        pub fn style(&self, id: &str, value: Style) {
            self.0.borrow_mut().styles.insert(id.into(), value);
        }
        pub fn label(&self, _: &Parent, id: &str, text: Binding<String>) {
            assert!(self.0.borrow_mut().texts.insert(id.into(), text).is_none());
        }
        pub fn entry(&self, p: &Parent, id: &str, text: Binding<String>) {
            self.label(p, id, text);
        }
        pub fn button(&self, p: &Parent, id: &str, text: Binding<String>, command: Command) {
            self.label(p, id, text);
            self.0.borrow_mut().commands.insert(id.into(), command);
        }
        pub fn meter(&self, _: &Parent, id: &str, value: Binding<f64>) {
            self.0.borrow_mut().values.insert(id.into(), value);
        }
        pub fn items<T: Clone + 'static>(
            &self,
            p: &Parent,
            id: &str,
            items: Binding<Vec<Keyed<T>>>,
            viewport: Binding<Viewport>,
            template: Template<T>,
        ) {
            let vp = viewport.get();
            self.0.borrow_mut().viewports.insert(id.into(), viewport);
            for item in items.get().into_iter().skip(vp.start).take(vp.count) {
                template(self, p, &format!("{id}/{}", item.key), item.model);
            }
        }
        pub fn content<T: Clone + 'static>(
            &self,
            p: &Parent,
            id: &str,
            item: Binding<Option<Keyed<T>>>,
            template: Template<T>,
        ) {
            if let Some(item) = item.get() {
                template(self, p, &format!("{id}/{}", item.key), item.model);
            }
        }
        pub fn tab_items<T: Clone + 'static>(
            &self,
            parent: &Parent,
            id: &str,
            items: Binding<Vec<Keyed<T>>>,
            _: Binding<String>,
            title: Rc<dyn Fn(Store<T>) -> Binding<String>>,
            template: Template<T>,
        ) -> Parent {
            for item in items.get() {
                let scope = format!("{id}/{}", item.key);
                self.label(parent, &scope, title(item.model.clone()));
                template(self, parent, &scope, item.model);
            }
            Parent
        }
        pub fn tab_position(&self, _: &str, _: TabPosition) -> Result<(), String> {
            Ok(())
        }
        pub fn typed_menu(
            &self,
            id: &str,
            _title: Binding<String>,
            items: Binding<Vec<MenuItem>>,
            dispatch: Rc<dyn Fn(String) -> bool>,
        ) {
            self.0.borrow_mut().menus.insert(id.into(), items);
            self.0.borrow_mut().dispatch.insert(id.into(), dispatch);
        }
        pub fn keep(&self, token: Subscription) {
            self.0.borrow_mut().tokens.push(token);
        }
        pub fn on_close(&self, close: Rc<dyn Fn()>) {
            self.0.borrow_mut().close = Some(close);
        }
        pub fn close(&self) {
            let close = self.0.borrow_mut().close.take();
            let tokens = std::mem::take(&mut self.0.borrow_mut().tokens);
            self.0.borrow_mut().dispatch.clear();
            drop(tokens);
            if let Some(close) = close {
                close();
            }
        }
    }
}
struct Exec(futures::executor::LocalSpawner);
impl Executor for Exec {
    fn spawn(&self, future: futures::future::LocalBoxFuture<'static, ()>) {
        self.0.spawn_local(future).unwrap();
    }
}
fn setup() -> (LocalPool, Store<Settings>, Store<Row>) {
    let pool = LocalPool::new();
    let exec: Rc<dyn Executor> = Rc::new(Exec(pool.spawner()));
    let leaf = Store::new(
        Leaf {
            name: "leaf".into(),
        },
        exec.clone(),
    );
    let row = Store::new(
        Row {
            name: "row".into(),
            error: String::new(),
            pending: false,
            progress: 0.5,
            children: vec![Keyed::new("child", leaf.clone())],
            selected: Some(Keyed::new("picked", leaf)),
        },
        exec.clone(),
    );
    let model = Store::new(
        Settings {
            title: "Title".into(),
            name: "initial".into(),
            argument: "first".into(),
            progress: 1.0,
            error: String::new(),
            pending: false,
            allow_save: false,
            calls: vec![],
            rows: vec![Keyed::new("key", row.clone())],
            selected: Some(Keyed::new("chosen", row.clone())),
            viewport: Viewport::default(),
            menu_items: vec![],
        },
        exec,
    );
    (pool, model, row)
}
#[test]
fn constructs_all_controls_templates_and_typed_bindings() {
    let (mut pool, model, row) = setup();
    let view = view!("tests/ui/main.yaml", model, fake);
    let texts = &view.0.borrow().texts;
    for id in [
        "name",
        "converted",
        "readonly",
        "caption",
        "rows/key/edit",
        "rows/key/outer",
        "rows/key/children/child/leaf",
        "rows/key/selected_child/picked/leaf",
        "selected/chosen/edit",
    ] {
        assert!(texts.contains_key(id), "{id}");
    }
    assert_eq!(texts["rows/key/outer"].get(), "Title");
    assert_eq!(texts["tabs/key"].get(), "row");
    assert_eq!(texts["tabs/key/leaf"].get(), "row");
    assert!(!texts["caption"].is_writable());
    assert!(!texts["readonly"].set("ignored".into()));
    assert!(texts["name"].set("edited".into()));
    assert_eq!(model.snapshot().name, "edited");
    assert!(texts["rows/key/edit"].set("row edited".into()));
    assert_eq!(row.snapshot().name, "row edited");
    assert_eq!(texts["tabs/key"].get(), "row edited");
    assert_eq!(texts["converted"].get(), "1");
    assert!(texts["converted"].set("2.5".into()));
    assert_eq!(model.snapshot().progress, 2.5);
    assert!(!texts["converted"].set("bad".into()));
    assert_eq!(model.snapshot().progress, 2.5);
    assert!(view.0.borrow().values["rows/key/meter"].set(0.75));
    assert_eq!(row.snapshot().progress, 0.75);
    assert_eq!(texts["rows/key/outer"].get(), "Title");
    assert!(matches!(
        view.0.borrow().layouts["rows/key/stack"],
        Layout::Stack {
            horizontal: true,
            gap: 2.0
        }
    ));
    assert!(matches!(
        view.0.borrow().layouts["rows/key/row"],
        Layout::Grid {
            columns: 2,
            gap: 3.0
        }
    ));
    assert!(matches!(
        view.0.borrow().layouts["absolute"],
        Layout::Absolute
    ));
    assert!(matches!(
        view.0.borrow().layouts["flex"],
        Layout::Flex { gap: 4.0 }
    ));
    assert_eq!(view.0.borrow().styles["flex"].placement.x, 10.0);
    assert_eq!(view.0.borrow().styles["absolute"].padding.left, 4.0);
    pool.run_until_stalled();
}
#[test]
fn window_style_targets_the_internal_root() {
    let (_pool, model, _) = setup();
    let view = view!("tests/ui/main.yaml", model, fake);
    let state = view.0.borrow();
    assert!(!state.styles.contains_key("main"));
    let style = &state.styles["__root"];
    assert_eq!(style.margin.top, 7.0);
    assert_eq!(style.margin.left, 7.0);
    assert_eq!(style.padding.top, 1.0);
    assert_eq!(style.padding.right, 2.0);
    assert_eq!(style.padding.bottom, 3.0);
    assert_eq!(style.padding.left, 4.0);
    assert_eq!(
        style.horizontal_alignment,
        Some(panechord_runtime::Alignment::End)
    );
    assert_eq!(
        style.vertical_alignment,
        Some(panechord_runtime::Alignment::Stretch)
    );
}
#[test]
fn items_and_bindable_layout_viewports_write_back_only_for_two_way() {
    let (_pool, model, _) = setup();
    let view = view!("tests/ui/main.yaml", model, fake);
    for (id, start, count, row_height) in [("rows", 3, 5, 24.0), ("viewport_items", 7, 9, 32.0)] {
        let viewport = view.0.borrow().viewports[id].clone();
        assert!(viewport.is_writable(), "{id}");
        assert!(viewport.set(Viewport {
            start,
            count,
            row_height,
        }));
        let value = model.snapshot().viewport;
        assert_eq!(value.start, start);
        assert_eq!(value.count, count);
        assert_eq!(value.row_height, row_height);
    }
    let readonly = view.0.borrow().viewports["rows/key/children"].clone();
    assert!(!readonly.is_writable());
    assert_eq!(readonly.get().start, 7);
    assert!(!readonly.set(Viewport::default()));
    assert_eq!(model.snapshot().viewport.start, 7);
}
#[test]
fn command_arguments_gates_static_actions_and_menu_are_live() {
    let (_pool, model, row) = setup();
    let owner = model
        .try_into_owner()
        .unwrap_or_else(|_| panic!("shared model"));
    let model = owner.store().clone();
    let view = owned_view!("tests/ui/main.yaml", owner, fake);
    let add = view.0.borrow().commands["add"].clone();
    let _ = model.update(|s| s.argument = "at-invocation".into());
    assert!(add.invoke());
    assert_eq!(model.snapshot().calls, ["at-invocation"]);
    assert!(view.0.borrow().commands["reset"].invoke());
    let save = view.0.borrow().commands["save"].clone();
    assert!(!save.enabled.get());
    assert!(!save.invoke());
    let _ = model.update(|s| s.allow_save = true);
    assert!(save.enabled.get());
    assert!(save.invoke());
    assert!(view.0.borrow().commands["rows/key/rename"].invoke());
    assert_eq!(row.snapshot().name, "at-invocation");
    assert!(view.0.borrow().commands["rows/key/outer_add"].invoke());
    let dispatch = view.0.borrow().dispatch["actions"].clone();
    let _ = model.update(|s| {
        s.menu_items.push(MenuItem {
            id: "copy".into(),
            label: "Copy".into(),
            argument: "payload".into(),
            shortcut: Some("Ctrl+C".into()),
            role: MenuRole::Copy,
            enabled: true,
        })
    });
    let item = view.0.borrow().menus["actions"].get().remove(0);
    assert_eq!(item.role, MenuRole::Copy);
    assert!(dispatch(item.argument));
    assert_eq!(
        model.snapshot().calls,
        [
            "at-invocation",
            "reset",
            "saved",
            "from-row",
            "menu:payload"
        ]
    );
    view.close();
    assert!(model.is_closed());
    assert!(!add.invoke());
}
#[test]
fn validation_tokens_are_kept_and_dropped() {
    let (mut pool, model, _) = setup();
    let view = view!("tests/ui/main.yaml", model, fake);
    assert_eq!(view.0.borrow().tokens.len(), 3);
    pool.run_until_stalled();
    assert!(!model.snapshot().pending);
    let entry = view.0.borrow().texts["name"].clone();
    let _ = entry.set(String::new());
    assert!(model.snapshot().pending);
    pool.run_until_stalled();
    assert_eq!(model.snapshot().error, "required");
    let _ = entry.set("ok".into());
    // Drop while a validation future is queued. No stale error can be applied.
    view.0.borrow_mut().tokens.clear();
    pool.run_until_stalled();
    assert!(!model.snapshot().pending);
    assert_eq!(model.snapshot().error, "");
}
#[test]
fn fixture_is_an_actual_cargo_tracked_input() {
    let source = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/ui/main.yaml"));
    assert!(source.contains("schema_version: 2"));
    panechord_markup::parse(source, "yaml", "tests/ui/main.yaml").unwrap();
}

#[allow(dead_code, non_upper_case_globals, unused_macros)]
mod shadowed {
    use super::{RowCommands, Settings, SettingsCommands, fake};
    use panechord_runtime::Store;
    struct String;
    struct Some;
    const None: u8 = 0;
    macro_rules! format {
        ($($t:tt)*) => {
            compile_error!("must use std::format!")
        };
    }
    #[derive(Clone)]
    pub struct ShadowModel;
    #[panechord_macros::commands]
    impl ShadowModel {
        #[command]
        fn reset(&mut self) {}
    }
    pub fn build(model: &Store<Settings>) -> fake::View {
        panechord_macros::view!("tests/ui/main.yaml", model, fake)
    }
}
#[test]
fn generated_names_do_not_depend_on_the_callers_prelude() {
    let (_pool, model, _) = setup();
    let view = shadowed::build(&model);
    assert!(
        view.0
            .borrow()
            .texts
            .contains_key("rows/key/children/child/leaf")
    );
}

#[test]
fn json_nested_field_writes_and_single_model_expression_evaluation() {
    #[derive(Clone)]
    struct Nested {
        nested: Leaf,
    }
    let pool = LocalPool::new();
    let model = Store::new(
        Nested {
            nested: Leaf {
                name: "before".into(),
            },
        },
        Rc::new(Exec(pool.spawner())),
    );
    let count = std::cell::Cell::new(0);
    let view = view!(
        "tests/ui/nested.json",
        {
            count.set(count.get() + 1);
            model.clone()
        },
        fake
    );
    assert_eq!(count.get(), 1);
    let text = view.0.borrow().texts["nested"].clone();
    assert_eq!(text.get(), "before");
    assert!(text.set("after".into()));
    assert_eq!(model.snapshot().nested.name, "after");
    assert_eq!(view.0.borrow().values["integer_literal"].get(), 1.0);
    assert_eq!(view.0.borrow().title.as_ref().unwrap().get(), "Nested JSON");
}

#[derive(Clone)]
struct SharedModel {
    name: String,
}

#[test]
fn shared_views_retain_model_until_both_close_without_closing_store() {
    let pool = LocalPool::new();
    let executor = Rc::new(Exec(pool.spawner()));
    let weak_executor = Rc::downgrade(&executor);
    let model = Store::new(
        SharedModel {
            name: "initial".into(),
        },
        executor,
    );
    let first = view!("tests/ui/lifecycle.yaml", model, fake);
    let second = view!("tests/ui/lifecycle.yaml", model, fake);
    let binding = second.0.borrow().texts["name"].clone();
    first.close();
    assert!(!model.is_closed());
    assert!(binding.set("after first close".into()));
    assert_eq!(model.snapshot().name, "after first close");
    drop(model);
    assert!(weak_executor.upgrade().is_some());
    assert!(binding.set("retained by second".into()));
    second.close();
    assert!(weak_executor.upgrade().is_none());
    assert!(!binding.set("no owners".into()));
    assert_eq!(binding.get(), "retained by second");
    // Closed View handles and their frozen bindings are deliberately still alive.
    first.close();
    second.close();
}

#[test]
fn shared_views_prevent_acquiring_an_owner_and_owned_close_is_explicit() {
    let pool = LocalPool::new();
    let model = Store::new(
        SharedModel {
            name: "initial".into(),
        },
        Rc::new(Exec(pool.spawner())),
    );
    let shared = view!("tests/ui/lifecycle.yaml", model, fake);
    let model = model
        .try_into_owner()
        .err()
        .expect("shared view must prevent ownership");
    let binding = shared.0.borrow().texts["name"].clone();
    assert!(binding.set("still shared".into()));
    shared.close();
    let owner = model
        .try_into_owner()
        .unwrap_or_else(|_| panic!("released shared view"));
    let model = owner.store().clone();
    let owned = owned_view!("tests/ui/lifecycle.yaml", owner, fake);
    owned.close();
    assert!(model.is_closed());
    assert!(!binding.set("closed".into()));
    assert_eq!(binding.get(), "still shared");

    let model = Store::new(
        SharedModel {
            name: "drop".into(),
        },
        Rc::new(Exec(pool.spawner())),
    );
    let first = view!("tests/ui/lifecycle.yaml", model, fake);
    let second = view!("tests/ui/lifecycle.yaml", model, fake);
    let binding = second.0.borrow().texts["name"].clone();
    drop(model);
    drop(first);
    assert!(binding.set("one owner".into()));
    drop(second);
    assert!(!binding.set("no owner".into()));
}

#[derive(Clone, PartialEq, Debug)]
enum Choice {
    First,
    Second,
}
#[derive(Clone)]
struct TypedModel {
    integer: i32,
    choice: Choice,
    calls: Vec<String>,
}
#[commands]
impl TypedModel {
    #[command]
    fn integer(&mut self, value: i32) {
        self.calls.push(format!("int:{value}"));
    }
    #[command]
    fn choice(model: Store<Self>, value: Choice) {
        let _ = model.update(move |s| s.calls.push(format!("enum:{value:?}")));
    }
    #[command]
    fn numeric(&mut self, value: f64) {
        self.calls.push(format!("float:{value}"));
    }
}

#[test]
fn typed_enum_integer_fields_and_f64_literal_commands_are_argument_erased() {
    let pool = LocalPool::new();
    let model = Store::new(
        TypedModel {
            integer: 1,
            choice: Choice::First,
            calls: vec![],
        },
        Rc::new(Exec(pool.spawner())),
    );
    let view = view!("tests/ui/typed.yaml", model, fake);
    let _ = model.update(|s| {
        s.integer = 42;
        s.choice = Choice::Second;
    });
    for id in ["integer", "choice", "numeric"] {
        let command: Command = view.0.borrow().commands[id].clone();
        assert!(command.invoke());
    }
    assert_eq!(
        model.snapshot().calls,
        ["int:42", "enum:Second", "float:2.5"]
    );
    view.close();
    assert!(!model.is_closed());
}

#[test]
fn closing_shared_view_does_not_cancel_accepted_effect() {
    let (mut pool, model, _) = setup();
    let view = view!("tests/ui/lifecycle.yaml", model, fake);
    assert!(model.effect(
        Policy::Parallel,
        "shared-work",
        async { String::from("finished") },
        |state, value| state.name = value,
    ));
    view.close();
    pool.run_until_stalled();
    assert_eq!(model.snapshot().name, "finished");
    assert!(!model.is_closed());
}

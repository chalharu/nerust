use std::{cell::RefCell, future::Future, pin::Pin, rc::Rc};
use wakui2_runtime::{Binding, Command, Executor, Layout, Store, Style};

#[derive(Clone)]
pub struct Model {
    pub name: String,
    pub flag: bool,
}

#[wakui2_macros::commands]
impl Model {
    #[command]
    fn reset(&mut self) {
        self.name.clear();
    }
    #[command]
    fn typed(&mut self, value: i32) {
        self.name = value.to_string();
    }
}

struct NoTasks;
impl Executor for NoTasks {
    fn spawn(&self, _: Pin<Box<dyn Future<Output = ()>>>) {
        panic!("these fixtures must not spawn tasks");
    }
}

pub fn model() -> Store<Model> {
    Store::new(
        Model {
            name: "model text".into(),
            flag: true,
        },
        Rc::new(NoTasks),
    )
}

// Only the methods emitted by these fixtures; no GTK/Tao or permissive generics.
pub mod fake {
    use super::*;

    #[derive(Clone)]
    pub struct Parent;
    pub struct View(RefCell<String>);

    impl View {
        pub fn new(_: Binding<String>) -> Self {
            Self(RefCell::new(String::new()))
        }
        pub fn root(&self) -> Parent {
            Parent
        }
        pub fn style(&self, _: &str, _: Style) {}
        pub fn layout(&self, _: &Parent, _: &str, _: Layout) -> Parent {
            Parent
        }
        pub fn label(&self, _: &Parent, _: &str, text: Binding<String>) {
            *self.0.borrow_mut() = text.get();
        }
        pub fn entry(&self, _: &Parent, _: &str, _: Binding<String>) {}
        pub fn button(&self, _: &Parent, _: &str, _: Binding<String>, _: Command) {}
        pub fn on_close(&self, _: Rc<dyn Fn()>) {}
        pub fn output(&self) -> String {
            self.0.borrow().clone()
        }
    }
}

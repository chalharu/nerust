use std::{future::Future, pin::Pin, rc::Rc};
use wakui2_macros::commands;
use wakui2_runtime::{Binding, Executor, Store};

#[derive(Clone, Debug, PartialEq)]
struct Model(i32);

trait Payload {
    type Value;
}
impl Payload for Model {
    type Value = Option<Vec<Model>>;
}

#[commands]
impl Model {
    const SIZE: usize = 2;

    #[command]
    fn replace(&mut self, other: Self) {
        *self = other;
    }
    #[command]
    fn optional(&mut self, other: Option<Self>) {
        *self = other.unwrap();
    }
    #[command]
    fn vector(&mut self, mut other: Vec<Self>) {
        *self = other.pop().unwrap();
    }
    #[command]
    fn nested(&mut self, mut other: Option<Vec<Self>>) {
        *self = other.as_mut().unwrap().pop().unwrap();
    }
    #[command]
    fn associated(&mut self, mut other: <Self as Payload>::Value) {
        *self = other.as_mut().unwrap().pop().unwrap();
    }
    #[command]
    fn static_replace(model: Store<Self>, other: Self) {
        let _ = model.update(move |state| *state = other);
    }
    #[command]
    fn static_nested(model: Store<Self>, mut other: Option<Vec<Self>>) {
        let _ = model.update(move |state| *state = other.as_mut().unwrap().pop().unwrap());
    }
    #[command]
    fn concrete(&mut self, other: Model) {
        *self = other;
    }
    #[command]
    fn sized(&mut self, other: [u8; Self::SIZE]) {
        self.0 = i32::from(other[0]);
    }
    #[command]
    fn static_associated(model: Store<Self>, mut other: <Self as Payload>::Value) {
        let _ = model.update(move |state| *state = other.as_mut().unwrap().pop().unwrap());
    }
}

struct NoTasks;
impl Executor for NoTasks {
    fn spawn(&self, _: Pin<Box<dyn Future<Output = ()>>>) {
        panic!("these commands must not spawn tasks");
    }
}

pub fn main() {
    // Original inherent signatures must keep their model-context Self too.
    let mut original = Model(0);
    original.replace(Model(1));
    original.nested(Some(vec![Model(2)]));
    assert_eq!(original, Model(2));

    let model = Store::new(Model(0), Rc::new(NoTasks));
    let commands = [
        model.command_replace(Binding::constant(Model(1))),
        model.command_optional(Binding::constant(Some(Model(2)))),
        model.command_vector(Binding::constant(vec![Model(3)])),
        model.command_nested(Binding::constant(Some(vec![Model(4)]))),
        model.command_associated(Binding::constant(Some(vec![Model(5)]))),
        model.command_static_replace(Binding::constant(Model(6))),
        model.command_static_nested(Binding::constant(Some(vec![Model(7)]))),
        model.command_concrete(Binding::constant(Model(8))),
        model.command_sized(Binding::constant([9, 0])),
        model.command_static_associated(Binding::constant(Some(vec![Model(10)]))),
    ];
    for (index, command) in commands.iter().enumerate() {
        assert!(command.invoke());
        assert_eq!(model.snapshot(), Model(index as i32 + 1));
    }
    println!("self-arguments-ok");
}

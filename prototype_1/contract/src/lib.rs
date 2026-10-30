//! Host-compatible view contracts. No UI, runtime, native handles or reflection.
use std::marker::PhantomData;

/// A state snapshot and the typed UI inputs accepted by its owner.
pub trait Contract: 'static {
    type State: Clone + 'static;
    type Input: 'static;
    fn descriptor() -> Descriptor;
}

/// Typed accessors are shared between the compiler metadata and the target app.
pub struct PropertyKey<C: Contract, T> {
    pub name: &'static str,
    pub read: fn(&C::State) -> T,
    pub write: Option<fn(T) -> C::Input>,
}

pub struct CommandKey<C: Contract> {
    pub name: &'static str,
    pub input: fn() -> C::Input,
    pub enabled: fn(&C::State) -> bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueType {
    Text,
    Bool,
    Items {
        contract: &'static str,
        key: &'static str,
        text: &'static str,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListItem {
    pub key: String,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct PropertyDescriptor {
    pub name: &'static str,
    /// Trusted Rust key exported by the shared contract, never UI input.
    pub rust_key: &'static str,
    pub value_type: ValueType,
    pub writable: bool,
}

#[derive(Debug, Clone)]
pub struct CommandDescriptor {
    pub name: &'static str,
    pub rust_key: &'static str,
}

#[derive(Debug, Clone)]
pub struct Descriptor {
    pub name: &'static str,
    pub properties: Vec<PropertyDescriptor>,
    pub commands: Vec<CommandDescriptor>,
}

/// A compiler marker records the selected backend in the generated API.
pub struct BackendMarker<B>(pub PhantomData<B>);

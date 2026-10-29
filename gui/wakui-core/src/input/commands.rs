//! Framework command vocabulary: opaque identity plus an extensible spec.
//!
//! The same direction as [`crate::scene::control`]: new commands are added as
//! `impl CommandSpec` outside this file, never as new enum variants here.
//! `Command` carries a boxed spec plus an `enabled` flag; native adapters
//! forward opaque [`CommandRef`] activations and resolve meaning view-side
//! against the latest model, like [`crate::scene::control::ControlEvent`].
//!
//! Two base specs cover the framework primitives: [`UnitSpec`] (no argument)
//! and [`IndexedSpec`] (one `u64` argument, e.g. an item index chosen
//! view-side). Flavors downcast via `as_any`, never via domain types or
//! debug names.

use std::any::Any;

use crate::scene::control::Id;

/// Process-unique opaque command identity. Minted view-side from a stable
/// domain key via [`crate::scene::control::IdRegistry`]; never a position or label.
pub type CommandId = Id;

/// Extensible command shape. Only `clone_box` is mandatory; `equals` degrades
/// to a safe default (always different, so a missing override stays correct
/// and only refreshes more). Domain commands implement this trait outside
/// `wakui`; flavors dispatch via `as_any`, never via domain types.
pub trait CommandSpec: std::fmt::Debug + Send + 'static {
    fn clone_box(&self) -> Box<dyn CommandSpec>;
    fn equals(&self, other: &dyn CommandSpec) -> bool {
        let _ = other;
        false
    }
    fn as_any(&self) -> &dyn Any;
    /// Diagnostics only. Dispatch via `as_any` + `TypeId`, never this string.
    fn debug_name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }
    /// Framework-primitive argument. `None` is a unit command; `Some` is an
    /// indexed command. Domain meaning of the index lives view-side.
    fn argument(&self) -> Option<u64> {
        None
    }
}

/// Unit command: no argument.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UnitSpec;

impl CommandSpec for UnitSpec {
    fn clone_box(&self) -> Box<dyn CommandSpec> {
        Box::new(*self)
    }
    fn equals(&self, other: &dyn CommandSpec) -> bool {
        other.as_any().downcast_ref::<Self>() == Some(self)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Indexed command: one `u64` argument. The framework never interprets the
/// index; view definitions map it (e.g. to an item id).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IndexedSpec {
    pub index: u64,
}

impl CommandSpec for IndexedSpec {
    fn clone_box(&self) -> Box<dyn CommandSpec> {
        Box::new(*self)
    }
    fn equals(&self, other: &dyn CommandSpec) -> bool {
        other.as_any().downcast_ref::<Self>() == Some(self)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn argument(&self) -> Option<u64> {
        Some(self.index)
    }
}

/// Opaque activation: what native adapters forward. The view side resolves
/// the id against its latest [`CommandModel`] plus argument shape, exactly
/// like views resolve [`crate::scene::control::ControlId`] via `BindingTable`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandRef {
    pub id: CommandId,
    pub argument: Option<u64>,
}

impl CommandRef {
    pub fn unit(id: CommandId) -> Self {
        Self { id, argument: None }
    }

    pub fn indexed(id: CommandId, argument: u64) -> Self {
        Self {
            id,
            argument: Some(argument),
        }
    }
}

/// One declared command: stable identity, enablement, and an extensible spec.
/// The spec template carries the argument *shape* (`None` vs `Some`);
/// concrete arguments travel in [`CommandRef`], never here.
#[derive(Debug)]
pub struct Command {
    pub id: CommandId,
    pub enabled: bool,
    pub spec: Box<dyn CommandSpec>,
}

impl Clone for Command {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            enabled: self.enabled,
            spec: self.spec.clone_box(),
        }
    }
}

impl PartialEq for Command {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.enabled == other.enabled
            && self.spec.equals(other.spec.as_ref())
    }
}

impl Eq for Command {}

impl Command {
    /// Downcast the spec to a concrete framework-provided or external type.
    /// Dispatch via `TypeId`, never via domain types or debug names.
    pub fn downcast<T: 'static>(&self) -> Option<&T> {
        self.spec.as_any().downcast_ref::<T>()
    }

    /// Whether this declaration accepts the activation: same id and same
    /// argument shape. View-side validity (e.g. item existence) stays view-side.
    pub fn accepts(&self, activation: CommandRef) -> bool {
        self.id == activation.id && self.spec.argument().is_some() == activation.argument.is_some()
    }
}

/// All declarations for view-side model builders.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommandModel {
    pub definitions: Vec<Command>,
}

impl CommandModel {
    pub fn get(&self, id: CommandId) -> Option<&Command> {
        self.definitions.iter().find(|command| command.id == id)
    }

    pub fn visit(&self, visitor: &mut dyn CommandVisitor) {
        for command in &self.definitions {
            visitor.command(command);
        }
    }
}

/// Owned outcome of executing a command: what native adapters observe.
/// Produced view-side from an [`EventCommand`](crate::input::event::EventCommand);
/// produced by the same layer that owns [`Command`], never by the transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandEffect<K: Copy + Eq + std::hash::Hash> {
    /// Open a file (values like ROM wording arrive via [`crate::chrome::dialog::FileDialogTitles`]).
    OpenFile,
    OpenAux(K),
    About,
    Quit,
    ToggleFullscreen,
    LiveTick,
}

pub trait CommandVisitor {
    fn command(&mut self, command: &Command);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn specs_are_object_safe_and_storable() {
        let boxed: Box<dyn CommandSpec> = Box::new(IndexedSpec { index: 7 });
        assert_eq!(boxed.argument(), Some(7));
        let cloned = boxed.clone_box();
        assert!(cloned.as_any().downcast_ref::<IndexedSpec>().is_some());
        assert!(boxed.equals(cloned.as_ref()));
        let unit = UnitSpec;
        assert!(!boxed.equals(&unit));
        assert_eq!(unit.argument(), None);
    }

    #[test]
    fn accepts_matches_id_and_shape_only() {
        let id = CommandId::fresh();
        let command = Command {
            id,
            enabled: true,
            spec: Box::new(IndexedSpec { index: 0 }),
        };
        assert!(command.accepts(CommandRef::indexed(id, 42)));
        assert!(!command.accepts(CommandRef::unit(id)));
        assert!(!command.accepts(CommandRef::indexed(CommandId::fresh(), 42)));
    }
}

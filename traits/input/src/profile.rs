//! Controller profiles, slot assignments, and the input factory plan.

use std::{collections::HashMap, rc::Rc};

use super::{
    devices::ControllerCollection,
    events::{InputStateBuffer, InputValue},
    ids::{AttachmentId, ControlId, DeviceKindId, ProfileId},
};

/// A set of port identifiers a controller can occupy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortSet {
    pub ports: &'static [AttachmentId],
}

/// Identifies a single slot on the system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlotInfo {
    pub id: AttachmentId,
    pub label: &'static str,
}

impl SlotInfo {
    /// Check whether this slot corresponds to the given persisted ID string.
    pub fn matches_id(&self, id: &str) -> bool {
        self.id.as_str() == id
    }
}

/// Describes one control on a controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlInfo {
    pub id: ControlId,
    pub label: &'static str,
    pub kind: ControlKind,
    pub abstract_key: Option<AbstractKey>,
}

/// Classification of a control's physical behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlKind {
    Digital,
    Analog,
    AnalogStick { clickable: bool },
    Mouse,
}

/// System-agnostic logical key identifier for default binding resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AbstractKey {
    Button1,
    Button2,
    Button3,
    Button4,
    Button5,
    Button6,
    Button7,
    Button8,
    Select,
    Start,
    Guide,
    DpadUp,
    DpadDown,
    DpadLeft,
    DpadRight,
    Axis1X,
    Axis1Y,
    Axis2X,
    Axis2Y,
}

/// Describes one controller type (metadata sent to Frontend).
pub trait ControllerProfile: std::fmt::Debug + Send + Sync {
    fn profile_id(&self) -> ProfileId;
    fn label(&self) -> &'static str;
    fn port_sets(&self) -> &[PortSet];
    fn port_groups(&self) -> &[&[ControlInfo]];

    /// Device kind identifier for a given port group index.
    /// Override when port groups in one profile are different device kinds.
    fn device_kind_for_group(&self, _group_index: usize) -> DeviceKindId {
        DeviceKindId::new(self.profile_id().as_str())
    }
}

/// System port layout query. Factory → Frontend.
/// `slots()`/`controllers()` describe init-time topology: build once and cache.
pub trait InputPorts: std::fmt::Debug {
    fn slots(&self) -> &[SlotInfo];
    /// All controller profiles this system supports.
    /// Init-only: call during setup and cache; never poll on the hot path.
    fn controllers(&self) -> Box<dyn Iterator<Item = Rc<dyn ControllerProfile>> + '_>;

    /// Resolve a persisted slot ID string to an AttachmentId.
    /// Returns None if no slot matches.
    fn resolve_slot(&self, id: &str) -> Option<AttachmentId> {
        self.slots().iter().find(|s| s.matches_id(id)).map(|s| s.id)
    }

    /// Resolve a persisted controller ID string to a ControllerProfile.
    fn resolve_controller(&self, id: &str) -> Option<Rc<dyn ControllerProfile>> {
        self.controllers().find(|p| p.profile_id().as_str() == id)
    }
}

/// Slot-to-controller assignments.
/// Equality is order-sensitive and structural (slot order + [`ProfileId`] compare).
#[derive(Clone)]
pub struct InputAssignments {
    slots: Vec<(AttachmentId, Option<Rc<dyn ControllerProfile>>)>,
}

impl InputAssignments {
    pub fn new(slots: Vec<(AttachmentId, Option<Rc<dyn ControllerProfile>>)>) -> Self {
        Self { slots }
    }

    pub fn slots(&self) -> &[(AttachmentId, Option<Rc<dyn ControllerProfile>>)] {
        &self.slots
    }

    pub fn slots_mut(&mut self) -> &mut [(AttachmentId, Option<Rc<dyn ControllerProfile>>)] {
        &mut self.slots
    }

    /// Explicit order-sensitive structural comparison used by `PartialEq`.
    pub fn eq_ordered(&self, other: &Self) -> bool {
        self.slots.len() == other.slots.len()
            && self.slots.iter().zip(other.slots.iter()).all(|(a, b)| {
                a.0 == b.0
                    && match (&a.1, &b.1) {
                        (None, None) => true,
                        (Some(x), Some(y)) => x.profile_id() == y.profile_id(),
                        _ => false,
                    }
            })
    }
}

impl std::fmt::Debug for InputAssignments {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InputAssignments")
            .field(
                "slots",
                &self
                    .slots
                    .iter()
                    .map(|(s, c)| (s, c.as_ref().map(|p| p.profile_id())))
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

/// Order-sensitive structural equality: slot ids and controller profile ids
/// compared pairwise; the persisted projection must preserve slot order.
impl PartialEq for InputAssignments {
    fn eq(&self, other: &Self) -> bool {
        self.eq_ordered(other)
    }
}

impl Eq for InputAssignments {}

/// Errors from create_split.
/// All identifiers are typed; empty yields `ControllerNotFound`, wrong count yields `IncompatibleController`.
#[derive(Debug, Clone, thiserror::Error)]
pub enum CreateSplitError {
    #[error("slot '{slot}' not found")]
    SlotNotFound { slot: AttachmentId },
    #[error("controller '{controller}' not found")]
    ControllerNotFound { controller: ProfileId },
    #[error("controller '{controller}' cannot be assigned to slot '{slot}'")]
    IncompatibleController {
        controller: ProfileId,
        slot: AttachmentId,
    },
    #[error("slot conflict between {a} and {b}")]
    SlotConflict { a: AttachmentId, b: AttachmentId },
}

/// Factory for creating input runtime state.
pub trait InputSystemFactory: InputPorts + std::fmt::Debug {
    fn default_assignments(&self) -> InputAssignments;
    fn create_split(
        &self,
        controllers: &ControllerCollection,
    ) -> Result<InputResources, CreateSplitError>;
}

/// Init-time factory output: field map plus a buffer factory.
/// Thread endpoints live behind `nerust_emu_thread::InputPort`; this stays thread-free.
pub struct InputResources {
    field_map: HashMap<(AttachmentId, ControlId), usize>,
    new_buffer: Box<dyn Fn() -> Box<dyn InputStateBuffer> + Send + Sync>,
}

impl std::fmt::Debug for InputResources {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InputResources")
            .field("field_map", &self.field_map)
            .finish_non_exhaustive()
    }
}

impl InputResources {
    pub fn new(
        field_map: HashMap<(AttachmentId, ControlId), usize>,
        new_buffer: Box<dyn Fn() -> Box<dyn InputStateBuffer> + Send + Sync>,
    ) -> Self {
        Self {
            field_map,
            new_buffer,
        }
    }

    pub fn field_map(&self) -> &HashMap<(AttachmentId, ControlId), usize> {
        &self.field_map
    }

    pub fn field_map_mut(&mut self) -> &mut HashMap<(AttachmentId, ControlId), usize> {
        &mut self.field_map
    }

    pub fn new_buffer(&self) -> &(dyn Fn() -> Box<dyn InputStateBuffer> + Send + Sync) {
        &*self.new_buffer
    }

    pub fn take_factory(self) -> Box<dyn Fn() -> Box<dyn InputStateBuffer> + Send + Sync> {
        self.new_buffer
    }

    /// Write one absolute field through a fresh buffer (init-time helper).
    pub fn probe_set(
        &self,
        field: usize,
        value: InputValue,
    ) -> Result<(), super::events::BufferError> {
        let mut buf = (self.new_buffer)();
        buf.set(field, value)
    }
}

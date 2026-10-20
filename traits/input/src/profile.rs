//! Controller profiles, slot assignments, and the GUI/emu split runtime.

use std::{
    collections::HashMap,
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use super::{
    devices::ControllerCollection,
    events::{BufferError, InputStateBuffer, InputValue},
    ids::{AttachmentId, DeviceKindId, ProfileId},
    topology::ControlId,
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

/// Output of create_split.
/// `field_map` uses the standard `HashMap` hasher: init-time only, no extra dependency.
#[derive(Debug)]
pub struct InputResources {
    split: InputSplit,
    field_map: HashMap<(AttachmentId, ControlId), usize>,
}

impl InputResources {
    pub fn new(split: InputSplit, field_map: HashMap<(AttachmentId, ControlId), usize>) -> Self {
        Self { split, field_map }
    }

    pub fn split(&self) -> &InputSplit {
        &self.split
    }

    pub fn split_mut(&mut self) -> &mut InputSplit {
        &mut self.split
    }

    pub fn field_map(&self) -> &HashMap<(AttachmentId, ControlId), usize> {
        &self.field_map
    }

    pub fn field_map_mut(&mut self) -> &mut HashMap<(AttachmentId, ControlId), usize> {
        &mut self.field_map
    }
}

/// Thread-shared state reference.
/// `Send + Sync`: safe to share between the GUI producer and emu consumer threads.
pub struct InputSplit {
    shared: Arc<Mutex<Box<dyn InputStateBuffer>>>,
    flag: Arc<AtomicBool>,
    new_buffer: Box<dyn Fn() -> Box<dyn InputStateBuffer> + Send + Sync>,
}

impl InputSplit {
    pub fn new(
        shared: Arc<Mutex<Box<dyn InputStateBuffer>>>,
        flag: Arc<AtomicBool>,
        new_buffer: Box<dyn Fn() -> Box<dyn InputStateBuffer> + Send + Sync>,
    ) -> Self {
        Self {
            shared,
            flag,
            new_buffer,
        }
    }

    pub fn shared(&self) -> &Arc<Mutex<Box<dyn InputStateBuffer>>> {
        &self.shared
    }

    pub fn flag(&self) -> &Arc<AtomicBool> {
        &self.flag
    }

    pub fn new_buffer(&self) -> &(dyn Fn() -> Box<dyn InputStateBuffer> + Send + Sync) {
        &*self.new_buffer
    }
}

impl std::fmt::Debug for InputSplit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InputSplit")
            .field("shared", &self.shared)
            .field("flag", &self.flag)
            .finish_non_exhaustive()
    }
}

/// GUI thread input delivery.
/// `Send` but `!Sync`: keep each `GuiInput` on its owning GUI thread.
#[derive(Debug)]
pub struct GuiInput {
    shared: Arc<Mutex<Box<dyn InputStateBuffer>>>,
    flag: Arc<AtomicBool>,
    state: Box<dyn InputStateBuffer>,
    write_buf: Box<dyn InputStateBuffer>,
}

impl GuiInput {
    pub fn new(
        shared: Arc<Mutex<Box<dyn InputStateBuffer>>>,
        flag: Arc<AtomicBool>,
        new_buffer: Box<dyn Fn() -> Box<dyn InputStateBuffer> + Send + Sync>,
    ) -> Self {
        Self {
            shared,
            flag,
            state: (new_buffer)(),
            write_buf: (new_buffer)(),
        }
    }

    pub fn from_split(split: &InputSplit) -> Self {
        Self {
            shared: Arc::clone(split.shared()),
            flag: Arc::clone(split.flag()),
            state: (split.new_buffer())(),
            write_buf: (split.new_buffer())(),
        }
    }

    /// Write one absolute field of the staged GUI state.
    pub fn set(&mut self, field: usize, value: InputValue) -> Result<(), BufferError> {
        self.state.set(field, value)
    }

    /// Must be called every frame. Copies absolute state into shared.
    pub fn publish(&mut self) {
        // Both buffers come from the same factory; a mismatch is a programming
        // error: panic loudly instead of silently dropping input.
        if let Err(error) = self.write_buf.copy_state(&*self.state) {
            unreachable!("GuiInput buffers must share a concrete type: {error}");
        }
        // Swap into shared (fast pointer exchange). The mutex cannot be
        // poisoned: only std::mem::swap (non-panicking) runs under the lock.
        let mut lock = self
            .shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        std::mem::swap(&mut *lock, &mut self.write_buf);
        self.flag.store(true, Ordering::Release);
    }

    pub fn clear(&mut self) {
        self.state.clear();
    }
}

/// Emu thread input consumer.
/// `Send` but `!Sync`: keep each `EmuInput` on its owning emu thread.
#[derive(Debug)]
pub struct EmuInput {
    shared: Arc<Mutex<Box<dyn InputStateBuffer>>>,
    flag: Arc<AtomicBool>,
    read_buf: Box<dyn InputStateBuffer>,
}

impl EmuInput {
    pub fn new(
        shared: Arc<Mutex<Box<dyn InputStateBuffer>>>,
        flag: Arc<AtomicBool>,
        new_buffer: Box<dyn Fn() -> Box<dyn InputStateBuffer> + Send + Sync>,
    ) -> Self {
        Self {
            shared,
            flag,
            read_buf: (new_buffer)(),
        }
    }

    pub fn from_split(split: &InputSplit) -> Self {
        Self {
            shared: Arc::clone(split.shared()),
            flag: Arc::clone(split.flag()),
            read_buf: (split.new_buffer())(),
        }
    }

    /// Borrow the latest emu-side snapshot without taking it.
    /// Cores downcast this to their concrete buffer type per frame.
    pub fn read_buffer(&self) -> &dyn InputStateBuffer {
        &*self.read_buf
    }

    /// Must be called every frame start. Takes latest input from GUI.
    pub fn take(&mut self) {
        if self.flag.swap(false, Ordering::Acquire) {
            // Only non-panicking `swap` runs under the lock, so the mutex
            // cannot poison; recover the guard defensively anyway.
            let mut lock = self
                .shared
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            std::mem::swap(&mut *lock, &mut self.read_buf);
        }
    }
}

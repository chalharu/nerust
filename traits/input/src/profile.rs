//! Controller profiles, slot assignments, and the GUI/emu split runtime.

use std::{
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use super::{
    devices::ControllerCollection,
    events::{BufferError, InputStateBuffer, InputValue},
    ids::{AttachmentId, DeviceKindId, DigitalControlId, ProfileId},
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
    ///
    /// Override when different port groups within the same profile
    /// represent different device kinds (e.g. NES Famicom P2 has
    /// a microphone that P1 doesn't).
    fn device_kind_for_group(&self, _group_index: usize) -> DeviceKindId {
        DeviceKindId::new(self.profile_id().as_str())
    }
}

/// System port layout query. Factory → Frontend.
///
/// `slots()` and `controllers()` describe static init-time topology:
/// build them once during session setup and cache the result. The
/// default `resolve_*` helpers linearly scan those lists, which is fine
/// at init time but must not run per frame.
pub trait InputPorts: std::fmt::Debug {
    fn slots(&self) -> &[SlotInfo];
    /// All controller profiles this system supports.
    ///
    /// Init-only: returns a freshly allocated `Vec` per call, so callers
    /// must invoke it during setup and cache the result rather than
    /// polling it on the hot path.
    fn controllers(&self) -> Vec<Rc<dyn ControllerProfile>>;

    /// Resolve a persisted slot ID string to an AttachmentId.
    /// Returns None if no slot matches.
    fn resolve_slot(&self, id: &str) -> Option<AttachmentId> {
        self.slots().iter().find(|s| s.matches_id(id)).map(|s| s.id)
    }

    /// Resolve a persisted controller ID string to a ControllerProfile.
    fn resolve_controller(&self, id: &str) -> Option<Rc<dyn ControllerProfile>> {
        self.controllers()
            .iter()
            .find(|p| p.profile_id().as_str() == id)
            .cloned()
    }
}

/// Slot-to-controller assignments.
/// Uses Rc<dyn ControllerProfile> so callers can inspect profile methods directly
/// without string-based lookups.
///
/// Equality is order-sensitive and structural: slots must appear in the same
/// order with equal attachment ids, and each paired controller is compared by
/// [`ProfileId`] (not by pointer identity). Persisting an `InputAssignments`
/// must therefore preserve slot order for round-trip equality to hold.
#[derive(Clone)]
pub struct InputAssignments {
    pub slots: Vec<(AttachmentId, Option<Rc<dyn ControllerProfile>>)>,
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
/// compared pairwise in order. The persistable (stringly-typed) projection of
/// these assignments lives in `SessionHandle::current_assignments_pairs`
/// (gui shell); that projection must preserve slot order for round-trip
/// equality to hold.
impl PartialEq for InputAssignments {
    fn eq(&self, other: &Self) -> bool {
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

impl Eq for InputAssignments {}

/// Errors from create_split.
#[derive(Debug, Clone, thiserror::Error)]
pub enum CreateSplitError {
    #[error("slot '{slot}' not found")]
    SlotNotFound { slot: String },
    #[error("controller '{controller}' not found")]
    ControllerNotFound { controller: String },
    #[error("controller '{controller}' cannot be assigned to slot '{slot}'")]
    IncompatibleController { controller: String, slot: String },
    #[error("slot conflict between {a} and {b}")]
    SlotConflict { a: String, b: String },
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
#[derive(Debug)]
pub struct InputResources {
    pub split: InputSplit,
    /// (attachment, control) → absolute field index
    pub field_map: std::collections::HashMap<(AttachmentId, DigitalControlId), usize>,
}

/// Thread-shared state reference.
pub struct InputSplit {
    pub shared: Arc<Mutex<Box<dyn InputStateBuffer>>>,
    pub flag: Arc<AtomicBool>,
    pub new_buffer: Box<dyn Fn() -> Box<dyn InputStateBuffer> + Send + Sync>,
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
/// Maintains absolute input state in an internal buffer.
/// On publish, copies state → write_buf → shared (swap), preserving held keys.
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
        Self::from_split(&InputSplit {
            shared,
            flag,
            new_buffer,
        })
    }

    pub fn from_split(split: &InputSplit) -> Self {
        Self {
            shared: Arc::clone(&split.shared),
            flag: Arc::clone(&split.flag),
            state: (split.new_buffer)(),
            write_buf: (split.new_buffer)(),
        }
    }

    /// Write one absolute field of the staged GUI state.
    pub fn set(&mut self, field: usize, value: InputValue) -> Result<(), BufferError> {
        self.state.set(field, value)
    }

    /// Must be called every frame. Copies absolute state into shared.
    pub fn publish(&mut self) {
        // Prepare write_buf from absolute state (fast concrete copy).
        // Both buffers come from the same `new_buffer` factory, so a
        // mismatch here is a programming error; surface it loudly in
        // debug builds but keep release behavior total.
        let copy = self.write_buf.copy_state(&*self.state);
        debug_assert!(copy.is_ok(), "GuiInput buffers must share a concrete type");
        let _ = copy;
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
        Self::from_split(&InputSplit {
            shared,
            flag,
            new_buffer,
        })
    }

    pub fn from_split(split: &InputSplit) -> Self {
        Self {
            shared: Arc::clone(&split.shared),
            flag: Arc::clone(&split.flag),
            read_buf: (split.new_buffer)(),
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
            // Only std::mem::swap (non-panicking) runs under the lock,
            // so the mutex cannot be poisoned; recover the inner guard
            // defensively anyway.
            let mut lock = self
                .shared
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            std::mem::swap(&mut *lock, &mut self.read_buf);
        }
    }
}

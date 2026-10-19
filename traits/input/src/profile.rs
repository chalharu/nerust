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
    events::InputStateBuffer,
    ids::{AttachmentId, DigitalControlId, ProfileId},
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
    pub id: DigitalControlId,
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
    fn device_kind_for_group(&self, _group_index: usize) -> &'static str {
        self.profile_id().as_str()
    }
}

/// System port layout query. Factory → Frontend.
pub trait InputPorts: std::fmt::Debug {
    fn slots(&self) -> &[SlotInfo];
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

/// Structural equality: slot ids and controller profile ids in order.
/// No string allocation; the persistable projection lives in the
/// `From` impl below.
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
    pub new_buffer: Box<dyn Fn() -> Box<dyn InputStateBuffer>>,
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
/// Maintains absolute input state in `state` buffer.
/// On publish, copies state → write_buf → shared (swap), preserving held keys.
#[derive(Debug)]
pub struct GuiInput {
    shared: Arc<Mutex<Box<dyn InputStateBuffer>>>,
    flag: Arc<AtomicBool>,
    pub state: Box<dyn InputStateBuffer>,
    write_buf: Box<dyn InputStateBuffer>,
}

impl GuiInput {
    pub fn new(
        shared: Arc<Mutex<Box<dyn InputStateBuffer>>>,
        flag: Arc<AtomicBool>,
        new_buffer: Box<dyn Fn() -> Box<dyn InputStateBuffer>>,
    ) -> Self {
        let state = (new_buffer)();
        let write_buf = (new_buffer)();
        Self {
            shared,
            flag,
            state,
            write_buf,
        }
    }

    pub fn from_split(split: &InputSplit) -> Self {
        Self {
            shared: Arc::clone(&split.shared),
            flag: Arc::clone(&split.flag),
            state: (split.new_buffer)(),
            write_buf: (split.new_buffer)(),
        }
    }

    /// Must be called every frame. Copies absolute state into shared.
    pub fn publish(&mut self) {
        // Prepare write_buf from absolute state (fast concrete copy)
        self.write_buf.copy_state(&*self.state);
        // Swap into shared (fast pointer exchange).
        // SAFETY: The mutex is never poisoned because publish() only performs
        // std::mem::swap() and AtomicBool::store() while holding the lock,
        // neither of which can panic. copy_state() runs before the lock.
        let mut lock = self.shared.lock().unwrap();
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
    pub read_buf: Box<dyn InputStateBuffer>,
}

impl EmuInput {
    pub fn new(
        shared: Arc<Mutex<Box<dyn InputStateBuffer>>>,
        flag: Arc<AtomicBool>,
        new_buffer: Box<dyn Fn() -> Box<dyn InputStateBuffer>>,
    ) -> Self {
        let read_buf = (new_buffer)();
        Self {
            shared,
            flag,
            read_buf,
        }
    }

    pub fn from_split(split: &InputSplit) -> Self {
        Self {
            shared: Arc::clone(&split.shared),
            flag: Arc::clone(&split.flag),
            read_buf: (split.new_buffer)(),
        }
    }

    /// Must be called every frame start. Takes latest input from GUI.
    pub fn take(&mut self) {
        if self.flag.swap(false, Ordering::Acquire) {
            // SAFETY: take() only performs std::mem::swap() under the lock,
            // which never panics, so the mutex can never be poisoned.
            let mut lock = self.shared.lock().unwrap();
            std::mem::swap(&mut *lock, &mut self.read_buf);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PortId(&'static str);

impl PortId {
    pub const fn new(value: &'static str) -> Self {
        Self(value)
    }

    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

impl PartialEq<AttachmentId> for PortId {
    fn eq(&self, other: &AttachmentId) -> bool {
        self.0 == other.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AttachmentId(&'static str);

impl AttachmentId {
    pub const fn new(value: &'static str) -> Self {
        Self(value)
    }

    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

impl std::fmt::Display for AttachmentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DeviceKindId(&'static str);

impl DeviceKindId {
    pub const fn new(value: &'static str) -> Self {
        Self(value)
    }

    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DigitalControlId(&'static str);

impl DigitalControlId {
    pub const fn new(value: &'static str) -> Self {
        Self(value)
    }

    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

impl std::fmt::Display for DigitalControlId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

/// Identifies a controller profile type (e.g. "nes.standard_pad", "nes.famicom").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProfileId(&'static str);

impl ProfileId {
    pub const fn new(value: &'static str) -> Self {
        Self(value)
    }

    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

impl std::fmt::Display for ProfileId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AnalogControlId(&'static str);

impl AnalogControlId {
    pub const fn new(value: &'static str) -> Self {
        Self(value)
    }

    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ControlId {
    Digital(DigitalControlId),
    Analog(AnalogControlId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputTopologyDescriptor {
    pub ports: Vec<PortDescriptor>,
    pub devices: Vec<DeviceDescriptor>,
}

impl InputTopologyDescriptor {
    pub fn attachment(&self, id: AttachmentId) -> Option<&AttachmentSlotDescriptor> {
        self.ports
            .iter()
            .flat_map(|port| port.attachments.iter())
            .find(|attachment| attachment.id == id)
    }

    pub fn device(&self, kind: DeviceKindId) -> Option<&DeviceDescriptor> {
        self.devices.iter().find(|device| device.kind == kind)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortDescriptor {
    pub id: PortId,
    pub label: &'static str,
    pub attachments: Vec<AttachmentSlotDescriptor>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentSlotDescriptor {
    pub id: AttachmentId,
    pub label: &'static str,
    pub device: DeviceKindId,
    pub supported_devices: Vec<DeviceKindId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceDescriptor {
    pub kind: DeviceKindId,
    pub label: &'static str,
    pub controls: Vec<ControlDescriptor>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlDescriptor {
    Digital(DigitalControlDescriptor),
    Analog(AnalogControlDescriptor),
}

impl ControlDescriptor {
    pub const fn id(&self) -> ControlId {
        match self {
            Self::Digital(control) => ControlId::Digital(control.id),
            Self::Analog(control) => ControlId::Analog(control.id),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigitalControlDescriptor {
    pub id: DigitalControlId,
    pub label: &'static str,
    pub description: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalogControlDescriptor {
    pub id: AnalogControlId,
    pub label: &'static str,
    pub description: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DigitalInputState {
    Pressed,
    Released,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DigitalInputEvent {
    pub attachment: AttachmentId,
    pub control: DigitalControlId,
    pub state: DigitalInputState,
}

impl DigitalInputEvent {
    pub const fn new(
        attachment: AttachmentId,
        control: DigitalControlId,
        state: DigitalInputState,
    ) -> Self {
        Self {
            attachment,
            control,
            state,
        }
    }

    pub const fn pressed(attachment: AttachmentId, control: DigitalControlId) -> Self {
        Self::new(attachment, control, DigitalInputState::Pressed)
    }

    pub const fn released(attachment: AttachmentId, control: DigitalControlId) -> Self {
        Self::new(attachment, control, DigitalInputState::Released)
    }

    pub const fn is_pressed(self) -> bool {
        matches!(self.state, DigitalInputState::Pressed)
    }
}

// ===== New Input Architecture Types =====

use std::any::Any;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Result of a CPU bus read — data bits and which bits are valid.
#[derive(Debug, Copy, Clone)]
pub struct OpenBusReadResult {
    pub data: u8,
    pub mask: u8,
}

impl OpenBusReadResult {
    pub fn new(data: u8, mask: u8) -> Self {
        Self { data, mask }
    }
}

/// Port identifier used by controllers and hubs.
pub trait Port: std::fmt::Debug {
    fn index(&self) -> usize;
    fn id(&self) -> &'static str;

    /// Return the attachment ID for this port.
    /// The default implementation wraps `id()` in an `AttachmentId`.
    fn as_attachment_id(&self) -> AttachmentId {
        AttachmentId::new(self.id())
    }
}

/// A simple port implementation with numeric index and string id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SimplePort {
    index: usize,
    id: &'static str,
}

impl SimplePort {
    pub const fn new(index: usize, id: &'static str) -> Self {
        Self { index, id }
    }
}

impl Port for SimplePort {
    fn index(&self) -> usize {
        self.index
    }
    fn id(&self) -> &'static str {
        self.id
    }
}

/// Single physical controller (shift register logic).
pub trait Controller: std::fmt::Debug + Send {
    fn read(&mut self, port: &dyn Port) -> OpenBusReadResult;
    fn write(&mut self, port: &dyn Port, value: u8);
    fn sync_input(&mut self, _state: &[u8]) {}
    /// Return field map entries (attachment, control, field_index) for this
    /// controller at the given port. Default returns empty (no inputs).
    fn field_map(&self, _port: &dyn Port) -> Vec<(AttachmentId, DigitalControlId, usize)> {
        Vec::new()
    }
}

/// Multi-port controller hub routing reads/writes to per-port controllers.
pub trait ControllerHub: std::fmt::Debug + Send {
    fn read_port(&mut self, port: &dyn Port) -> OpenBusReadResult;
    fn write_strobe(&mut self, value: u8);
    fn sync_input(&mut self, state: &[u8]);
}

/// A collection of per-port controllers.
pub struct ControllerCollection {
    pub devices: Vec<Box<dyn Controller + Send>>,
}

impl std::fmt::Debug for ControllerCollection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ControllerCollection")
            .field("device_count", &self.devices.len())
            .finish_non_exhaustive()
    }
}

impl ControllerCollection {
    pub fn new(devices: Vec<Box<dyn Controller + Send>>) -> Self {
        Self { devices }
    }

    pub fn device_count(&self) -> usize {
        self.devices.len()
    }

    pub fn device_mut(&mut self, port: usize) -> Option<&mut Box<dyn Controller + Send>> {
        self.devices.get_mut(port)
    }

    pub fn iter_devices_mut(&mut self) -> impl Iterator<Item = &mut Box<dyn Controller + Send>> {
        self.devices.iter_mut()
    }
}

impl ControllerHub for ControllerCollection {
    fn read_port(&mut self, port: &dyn Port) -> OpenBusReadResult {
        self.devices
            .get_mut(port.index())
            .map_or_else(|| OpenBusReadResult::new(0, 0), |d| d.read(port))
    }
    fn write_strobe(&mut self, value: u8) {
        let port = SimplePort::new(0, "");
        for d in self.devices.iter_mut() {
            d.write(&port, value);
        }
    }
    fn sync_input(&mut self, state: &[u8]) {
        for d in &mut self.devices {
            d.sync_input(state);
        }
    }
}

unsafe impl Send for ControllerCollection {}

/// Values that can be written to an InputStateBuffer field.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InputValue {
    Digital(bool),
    Analog(f32),
    Position { x: f64, y: f64 },
}

/// Errors from InputStateBuffer operations.
#[derive(Debug, Clone, thiserror::Error)]
pub enum BufferError {
    #[error("field {field} not found")]
    FieldNotFound { field: usize },
    #[error("field {field} does not support value type {expected}")]
    UnsupportedFieldType {
        field: usize,
        expected: &'static str,
    },
}

/// GUI-side write abstraction for input state.
/// Emu side reads via `Any::downcast_ref`.
pub trait InputStateBuffer: std::fmt::Debug + Send + Any {
    /// field: 0..N の logical field index。値の意味は impl 定義。
    fn set(&mut self, field: usize, value: InputValue) -> Result<(), BufferError>;
    /// 全 field を neutral / released 状態にリセットする。impl 依存。
    fn clear(&mut self);
    /// Copy absolute state from another buffer of the same concrete type.
    fn copy_state(&mut self, other: &dyn InputStateBuffer);
}

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
        self.id.0 == id
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

impl InputAssignments {
    /// Convert to persistable string pairs (slot_id, option<controller_id>).
    pub fn to_string_pairs(&self) -> Vec<(String, Option<String>)> {
        self.slots
            .iter()
            .map(|(s, c)| {
                (
                    s.to_string(),
                    c.as_ref().map(|p| p.profile_id().to_string()),
                )
            })
            .collect()
    }
}

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
        // Swap into shared (fast pointer exchange)
        let mut lock = self.shared.lock().unwrap_or_else(|_| unreachable!());
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
            let mut lock = self.shared.lock().unwrap_or_else(|_| unreachable!());
            std::mem::swap(&mut *lock, &mut self.read_buf);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AnalogControlDescriptor, AnalogControlId, AttachmentId, AttachmentSlotDescriptor,
        ControlDescriptor, ControlId, DeviceDescriptor, DeviceKindId, DigitalControlDescriptor,
        DigitalControlId, DigitalInputEvent, DigitalInputState, InputTopologyDescriptor,
        PortDescriptor, PortId,
    };

    #[test]
    fn topology_tracks_ports_attachments_and_devices() {
        let attachment = AttachmentId::new("test.pad1");
        let device = DeviceKindId::new("test.gamepad");
        let topology = InputTopologyDescriptor {
            ports: vec![PortDescriptor {
                id: PortId::new("test.port1"),
                label: "Port 1",
                attachments: vec![AttachmentSlotDescriptor {
                    id: attachment,
                    label: "Player 1",
                    device,
                    supported_devices: vec![device],
                }],
            }],
            devices: vec![DeviceDescriptor {
                kind: device,
                label: "Gamepad",
                controls: vec![
                    ControlDescriptor::Digital(DigitalControlDescriptor {
                        id: DigitalControlId::new("test.a"),
                        label: "A",
                        description: "Primary face button",
                    }),
                    ControlDescriptor::Analog(AnalogControlDescriptor {
                        id: AnalogControlId::new("test.stick_x"),
                        label: "Stick X",
                        description: "Horizontal axis",
                    }),
                ],
            }],
        };

        assert_eq!(topology.attachment(attachment).unwrap().device, device);
        let controls = &topology.device(device).unwrap().controls;
        assert_eq!(
            controls[0].id(),
            ControlId::Digital(DigitalControlId::new("test.a"))
        );
        assert_eq!(
            controls[1].id(),
            ControlId::Analog(AnalogControlId::new("test.stick_x"))
        );
    }

    #[test]
    fn digital_input_event_helpers_preserve_state() {
        let attachment = AttachmentId::new("test.pad1");
        let control = DigitalControlId::new("test.a");

        assert!(DigitalInputEvent::pressed(attachment, control).is_pressed());
        assert_eq!(
            DigitalInputEvent::released(attachment, control).state,
            DigitalInputState::Released
        );
    }
}

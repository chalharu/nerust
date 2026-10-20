//! Physical controller ports, controllers, and multi-port hubs.

use super::{
    events::OpenBusReadResult,
    ids::{AttachmentId, PortId},
    topology::ControlId,
};

/// Port identifier used by controllers and hubs.
///
/// `Send + Sync` so port handles can cross the GUI/emu thread boundary
/// alongside the buffers they index.
pub trait Port: std::fmt::Debug + Send + Sync {
    fn index(&self) -> usize;
    fn id(&self) -> &'static str;
    /// Typed port identifier, derived from [`Port::id`] by default.
    fn port_id(&self) -> PortId {
        PortId::new(self.id())
    }
    /// Typed attachment identifier, derived from [`Port::id`] by default.
    ///
    /// Controllers must use this (not `AttachmentId::new(port.id())`) so
    /// the string aliasing stays in one place.
    fn attachment_id(&self) -> AttachmentId {
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
    fn port_id(&self) -> PortId {
        PortId::new(self.id)
    }
    fn attachment_id(&self) -> AttachmentId {
        AttachmentId::new(self.id)
    }
}

/// One resolved field entry: (attachment, control) → absolute field index.
///
/// Replaces the positional `(AttachmentId, DigitalControlId, usize)` tuple
/// so call sites name each component. `control` is the unified [`ControlId`]
/// so analog controls share the same map instead of needing a parallel
/// analog table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FieldEntry {
    pub attachment: AttachmentId,
    pub control: ControlId,
    pub field: usize,
}

impl FieldEntry {
    pub const fn new(attachment: AttachmentId, control: ControlId, field: usize) -> Self {
        Self {
            attachment,
            control,
            field,
        }
    }
}

/// Single physical controller (shift register logic).
pub trait Controller: std::fmt::Debug + Send {
    fn read(&mut self, port: &dyn Port) -> OpenBusReadResult;
    fn write(&mut self, port: &dyn Port, value: u8);
    /// Broadcast strobe write to this controller.
    ///
    /// Index 0 is the broadcast convention: hubs fan a single strobe
    /// value out to every attached controller. Each controller implements
    /// the latch-edge directly; there is no synthetic index-0 port.
    fn strobe(&mut self, value: u8);
    fn sync_input(&mut self, _state: &[u8]) {}
    /// Append field entries for this controller at the given port.
    /// Default appends nothing (no inputs).
    /// Implementations must only push; they must not clear `out`.
    fn field_map(&self, _port: &dyn Port, _out: &mut Vec<FieldEntry>) {}
}

/// Multi-port controller hub routing reads/writes to per-port controllers.
pub trait ControllerHub: std::fmt::Debug + Send {
    fn read_port(&mut self, port: &dyn Port) -> OpenBusReadResult;
    fn write_strobe(&mut self, value: u8);
    fn sync_input(&mut self, state: &[u8]);
}

/// A collection of per-port controllers.
pub struct ControllerCollection {
    devices: Vec<Box<dyn Controller + Send>>,
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

    pub fn len(&self) -> usize {
        self.devices.len()
    }

    pub fn is_empty(&self) -> bool {
        self.devices.is_empty()
    }

    /// The single indexed access: borrow a controller by collection index.
    pub fn get(&self, index: usize) -> Option<&(dyn Controller + Send)> {
        match self.devices.get(index) {
            Some(b) => Some(&**b),
            None => None,
        }
    }

    /// Borrow all controllers mutably (hub fan-out, input sync).
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut (dyn Controller + Send)> {
        self.devices
            .iter_mut()
            .map(|b| &mut **b as &mut (dyn Controller + Send))
    }
}

impl ControllerHub for ControllerCollection {
    fn read_port(&mut self, port: &dyn Port) -> OpenBusReadResult {
        self.devices
            .get_mut(port.index())
            .map_or_else(|| OpenBusReadResult::new(0, 0), |d| d.read(port))
    }
    fn write_strobe(&mut self, value: u8) {
        for d in self.devices.iter_mut() {
            d.strobe(value);
        }
    }
    fn sync_input(&mut self, state: &[u8]) {
        for d in &mut self.devices {
            d.sync_input(state);
        }
    }
}

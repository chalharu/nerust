//! Physical controller ports, controllers, and multi-port hubs.

use super::{
    events::OpenBusReadResult,
    ids::{AttachmentId, ControlId, PortId},
};

/// Port identifier used by controllers and hubs.
/// `Send + Sync` so handles cross the GUI/emu thread boundary.
pub trait Port: std::fmt::Debug + Send + Sync {
    fn index(&self) -> usize;
    fn id(&self) -> &'static str;
    /// Typed port identifier, derived from [`Port::id`] by default.
    fn port_id(&self) -> PortId {
        PortId::new(self.id())
    }
    /// Typed attachment identifier, derived from [`Port::id`] by default.
    /// Use this (not `AttachmentId::new`) so the aliasing stays in one place.
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
/// `control` is the unified [`ControlId`], so analog shares the same map.
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

/// Broadcast strobe endpoint, split from [`Controller`] reads.
/// Hubs fan one value out to every port; index 0 is the broadcast convention.
pub trait StrobeTarget: std::fmt::Debug + Send {
    /// Broadcast strobe write to this controller.
    fn strobe(&mut self, value: u8);
}

/// Single physical controller (shift register logic).
pub trait Controller: StrobeTarget {
    fn read(&mut self, port: &dyn Port) -> OpenBusReadResult;
    fn write(&mut self, port: &dyn Port, value: u8);
    fn sync_input(&mut self, _state: &[u8]) {}
    /// Append field entries for this controller at the given port.
    /// Default appends nothing; implementations must only push to `out`.
    fn field_map(&self, _port: &dyn Port, _out: &mut Vec<FieldEntry>) {}
    /// Owned field entries without an out-param.
    /// Default forwards to [`Controller::field_map`]; override for direct builds.
    fn field_entries(&self, port: &dyn Port) -> Vec<FieldEntry> {
        let mut out = Vec::new();
        self.field_map(port, &mut out);
        out
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

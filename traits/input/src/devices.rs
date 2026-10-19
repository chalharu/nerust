//! Physical controller ports, controllers, and multi-port hubs.

use super::{
    events::OpenBusReadResult,
    ids::{AttachmentId, DigitalControlId},
};

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

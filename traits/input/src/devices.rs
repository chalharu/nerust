//! Physical controller ports, controllers, and multi-port hubs.

use super::{
    events::OpenBusReadResult,
    ids::{AttachmentId, DigitalControlId},
};

/// Port identifier used by controllers and hubs.
///
/// `Send + Sync` so port handles can cross the GUI/emu thread boundary
/// alongside the buffers they index.
pub trait Port: std::fmt::Debug + Send + Sync {
    fn index(&self) -> usize;
    fn id(&self) -> &'static str;
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
    /// Broadcast strobe write to this controller.
    ///
    /// Index 0 is the broadcast convention: hubs fan a single strobe
    /// value out to every attached controller without naming a real port.
    /// The default implementation routes through [`Controller::write`]
    /// with a synthetic index-0 port; override for hardware where strobe
    /// is not an ordinary port write.
    fn strobe(&mut self, value: u8) {
        let port = SimplePort::new(0, "");
        self.write(&port, value);
    }
    fn sync_input(&mut self, _state: &[u8]) {}
    /// Append field map entries (attachment, control, field_index) for this
    /// controller at the given port. Default appends nothing (no inputs).
    /// Implementations must only push; they must not clear `out`.
    fn field_map(&self, _port: &dyn Port, _out: &mut Vec<(AttachmentId, DigitalControlId, usize)>) {
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

    pub fn get(&self, index: usize) -> Option<&(dyn Controller + Send)> {
        match self.devices.get(index) {
            Some(b) => Some(&**b),
            None => None,
        }
    }

    pub fn get_mut(&mut self, index: usize) -> Option<&mut (dyn Controller + Send)> {
        match self.devices.get_mut(index) {
            Some(b) => Some(&mut **b),
            None => None,
        }
    }

    pub fn device_count(&self) -> usize {
        self.devices.len()
    }

    pub fn device_mut(&mut self, port: &dyn Port) -> Option<&mut (dyn Controller + Send)> {
        let index = port.index();
        match self.devices.get_mut(index) {
            Some(b) => Some(&mut **b),
            None => None,
        }
    }

    pub fn iter_devices_mut(&mut self) -> impl Iterator<Item = &mut (dyn Controller + Send)> {
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

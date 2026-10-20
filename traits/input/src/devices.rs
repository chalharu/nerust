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

#[cfg(test)]
mod tests {
    use super::{
        Controller, ControllerCollection, ControllerHub, FieldEntry, Port, SimplePort, StrobeTarget,
    };
    use crate::{AttachmentId, ControlId, DigitalControlId, OpenBusReadResult};

    #[derive(Debug, Default)]
    struct ProbePad {
        strobe: u8,
        synced: Vec<u8>,
    }

    impl StrobeTarget for ProbePad {
        fn strobe(&mut self, value: u8) {
            self.strobe = value;
        }
    }

    impl Controller for ProbePad {
        fn read(&mut self, port: &dyn Port) -> OpenBusReadResult {
            OpenBusReadResult::new(port.index() as u8, 0x1F)
        }
        fn write(&mut self, _port: &dyn Port, value: u8) {
            self.strobe = value;
        }
        fn sync_input(&mut self, state: &[u8]) {
            self.synced = state.to_vec();
        }
        fn field_map(&self, port: &dyn Port, out: &mut Vec<FieldEntry>) {
            out.push(FieldEntry::new(
                port.attachment_id(),
                ControlId::digital(DigitalControlId::new("test.a")),
                port.index(),
            ));
        }
    }

    #[test]
    fn simple_port_derives_typed_ids() {
        let port = SimplePort::new(2, "test.p3");
        assert_eq!(port.index(), 2);
        assert_eq!(port.id(), "test.p3");
        assert_eq!(port.port_id().as_str(), "test.p3");
        assert_eq!(port.attachment_id(), AttachmentId::new("test.p3"));
    }

    #[test]
    fn field_entry_carries_attachment_control_and_index() {
        let entry = FieldEntry::new(
            AttachmentId::new("test.p1"),
            ControlId::digital(DigitalControlId::new("test.a")),
            4,
        );
        assert_eq!(entry.field, 4);
        assert_eq!(entry.control.as_str(), "test.a");
        assert_eq!(entry, FieldEntry::new(entry.attachment, entry.control, 4));
    }

    #[test]
    fn collection_routes_reads_and_fans_strobe() {
        let mut hubs = ControllerCollection::new(vec![
            Box::new(ProbePad::default()),
            Box::new(ProbePad::default()),
        ]);
        assert_eq!(hubs.len(), 2);
        assert!(!hubs.is_empty());
        assert!(hubs.get(9).is_none());
        assert!(hubs.get(0).is_some());
        let first = SimplePort::new(0, "test.p1");
        let missing = SimplePort::new(7, "test.px");
        assert_eq!(hubs.read_port(&first).data, 0);
        assert_eq!(hubs.read_port(&missing), OpenBusReadResult::new(0, 0));
        hubs.write_strobe(1);
        hubs.sync_input(&[0xAA]);
        for pad in hubs.iter_mut() {
            let _ = pad.read(&first);
        }
        let empty = ControllerCollection::new(vec![]);
        assert!(empty.is_empty());
        assert_eq!(empty.len(), 0);
    }

    #[test]
    fn controller_field_entries_default_to_empty() {
        #[derive(Debug)]
        struct Bare;
        impl StrobeTarget for Bare {
            fn strobe(&mut self, _value: u8) {}
        }
        impl Controller for Bare {
            fn read(&mut self, _port: &dyn Port) -> OpenBusReadResult {
                OpenBusReadResult::new(0, 0)
            }
            fn write(&mut self, _port: &dyn Port, _value: u8) {}
        }
        let bare = Bare;
        let port = SimplePort::new(0, "test.p1");
        assert!(bare.field_entries(&port).is_empty());
        let probe = ProbePad::default();
        assert_eq!(probe.field_entries(&port).len(), 1);
    }
}

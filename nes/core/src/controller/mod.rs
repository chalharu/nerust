use crate::OpenBusReadResult;

pub trait Controller {
    fn read(&mut self, port: usize) -> OpenBusReadResult;
    fn write(&mut self, port: usize, value: u8);
    fn sync_input(&mut self, _state: &[u8]) {}
}

/// Routes controller port reads/writes to per-port Controller instances.
pub trait ControllerHub {
    fn read_port(&mut self, port: usize) -> OpenBusReadResult;
    fn write_strobe(&mut self, value: u8);
    fn sync_input(&mut self, state: &[u8]);
}

pub struct ControllerCollection {
    devices: Vec<Box<dyn Controller + Send>>,
}

impl ControllerCollection {
    pub fn new(devices: Vec<Box<dyn Controller + Send>>) -> Self {
        Self { devices }
    }
}

impl ControllerHub for ControllerCollection {
    fn read_port(&mut self, port: usize) -> OpenBusReadResult {
        self.devices
            .get_mut(port)
            .map_or_else(|| OpenBusReadResult::new(0, 0), |d| d.read(port))
    }
    fn write_strobe(&mut self, value: u8) {
        for (port, d) in self.devices.iter_mut().enumerate() {
            d.write(port, value);
        }
    }
    fn sync_input(&mut self, state: &[u8]) {
        for d in &mut self.devices {
            d.sync_input(state);
        }
    }
}

unsafe impl Send for ControllerCollection {}

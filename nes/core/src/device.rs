pub trait Device: Send {
    fn kind(&self) -> DeviceKind;
    fn cycle(&mut self, io: &mut PortIo);
}

pub struct PortIo {
    pub device: DeviceKind,
    pub input: Vec<u8>,
    pub output: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceKind(pub u16);

impl DeviceKind {
    pub const NONE: Self = Self(0);
}

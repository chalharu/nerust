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
pub enum DeviceKind {
    None,
    NesPad,
    NesZapper,
    SnesPad,
    SnesMouse,
    GbLinkCable,
    Ps1MemoryCard,
    Ps1DualShock,
}

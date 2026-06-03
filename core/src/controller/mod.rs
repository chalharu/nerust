use crate::OpenBusReadResult;

pub trait Controller {
    fn read(&mut self, address: usize) -> OpenBusReadResult;
    fn write(&mut self, value: u8);
}

use crate::OpenBusReadResult;

pub trait Controller {
    fn read(&mut self, address: usize) -> OpenBusReadResult;
    fn write(&mut self, value: u8);
    /// Called before each frame to provide latest input state.
    /// Default impl is no-op.
    fn sync_input(&mut self, _state: &[u8]) {}
}

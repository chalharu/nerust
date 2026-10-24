use super::{SaveBackend, SaveType};

#[derive(Debug, Default)]
pub struct NoneSave;

impl NoneSave {
    pub fn new() -> Self {
        Self
    }
}

impl SaveBackend for NoneSave {
    fn save_type(&self) -> SaveType {
        SaveType::None
    }

    fn read(&self, _addr: u32, _width: u8) -> u32 {
        0xFFFFFFFF
    }

    fn write(&mut self, _addr: u32, _width: u8, _value: u32) {}

    fn ram_data(&self) -> Option<&[u8]> {
        None
    }

    fn ram_restore(&mut self, _data: &[u8]) {}

    fn serialize_state(&self) -> Vec<u8> {
        Vec::new()
    }

    fn deserialize_state(&mut self, _data: &[u8]) -> Result<(), String> {
        Ok(())
    }
}

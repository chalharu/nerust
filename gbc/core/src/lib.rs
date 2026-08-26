#[allow(dead_code)]
pub(crate) mod apu;
pub mod bootrom;
#[allow(dead_code)]
pub mod cartridge;
#[allow(dead_code)]
pub mod cartridge_header;
#[allow(dead_code)]
pub mod cartridge_mbc;
pub mod console_core;
pub mod core_options;
#[allow(dead_code)]
pub(crate) mod cpu;
#[allow(dead_code)]
pub mod cpu_core;
#[allow(dead_code)]
pub(crate) mod cpu_opcodes;
#[allow(dead_code)]
pub(crate) mod cpu_registers;
#[allow(dead_code)]
pub(crate) mod dma;
#[allow(dead_code)]
pub(crate) mod hdma;
pub mod input_types;
#[allow(dead_code)]
pub(crate) mod interrupt;
#[allow(dead_code)]
pub mod memory;
mod persistence;
mod persistence_error;
#[allow(dead_code)]
pub(crate) mod ppu;
pub mod rom_identity;
#[allow(dead_code)]
pub(crate) mod serial;
pub mod system;
#[allow(dead_code)]
pub(crate) mod timer;

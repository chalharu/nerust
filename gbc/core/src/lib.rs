#[allow(dead_code)]
pub(crate) mod apu;
pub mod bootrom;
#[allow(dead_code)]
pub mod cartridge;
#[allow(dead_code)]
pub mod cartridge_header;
#[allow(dead_code)]
pub mod cartridge_mbc;
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
pub(crate) mod interrupt;
#[allow(dead_code)]
pub mod memory;
#[allow(dead_code)]
pub(crate) mod ppu;
pub mod rom_identity;
#[allow(dead_code)]
pub(crate) mod serial;
#[allow(dead_code)]
pub(crate) mod timer;

mod rom_tests;

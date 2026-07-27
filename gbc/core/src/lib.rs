#![allow(dead_code)]

pub mod rom_identity;

pub(crate) mod apu;
pub(crate) mod cartridge;
pub(crate) mod dma;
pub(crate) mod memory;
pub(crate) mod ppu;
pub(crate) mod serial;
pub(crate) mod timer;

// Re-exported for GbcConsoleCore (Phase 9) and GbcFactory (Phase 10)
pub mod bootrom;

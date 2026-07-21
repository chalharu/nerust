use std::cell::{Ref, RefMut};

use nerust_core_traits::debugger::Debugger;
use nerust_core_traits::memory_space::MemorySpace;
use strum::IntoEnumIterator;

use super::memory_space::NesMemorySpace;
use crate::console_core::SendCore;

/// CPU register snapshot for debugger inspection.
#[derive(Debug, Clone, Copy, Default)]
pub struct NesCpuRegisters {
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub pc: u16,
    pub sp: u8,
    pub p: u8,
}

/// NES-specific debugger implementation.
///
/// Shares the internal `Core` with `NesConsoleCore` via `Rc<RefCell<>>`.
/// System-specific methods (cpu_registers, read_cartridge_ram, etc.)
/// are exposed as inherent methods — access them by downcasting from
/// `Box<dyn Debugger>`.
pub struct NesDebugger {
    core: SendCore,
    spaces: Box<[Box<dyn MemorySpace>]>,
}

impl NesDebugger {
    pub(crate) fn new(core: SendCore) -> Self {
        let spaces: Box<[Box<dyn MemorySpace>]> = NesMemorySpace::iter()
            .map(|s| Box::new(s) as Box<dyn MemorySpace>)
            .collect();
        Self { core, spaces }
    }

    fn core_ref(&self) -> Option<Ref<'_, crate::Core>> {
        let guard = self.core.borrow();
        if guard.is_some() {
            Some(Ref::map(guard, |o| o.as_ref().unwrap()))
        } else {
            None
        }
    }

    #[allow(dead_code)]
    fn core_mut(&self) -> Option<RefMut<'_, crate::Core>> {
        let guard = self.core.borrow_mut();
        if guard.is_some() {
            Some(RefMut::map(guard, |o| o.as_mut().unwrap()))
        } else {
            None
        }
    }
}

impl Debugger for NesDebugger {
    fn memory_spaces(&self) -> &[Box<dyn MemorySpace>] {
        &self.spaces
    }

    fn read(&self, space: &dyn MemorySpace, address: u32) -> Option<u8> {
        let core = self.core_ref()?;
        match space.id() {
            "cpu" => {
                let addr = address as usize;
                if addr < 0x2000 {
                    core.peek_work_ram(addr)
                } else if (0x6000..=0x7FFF).contains(&addr) {
                    core.peek_cartridge_ram(addr).map(|r| r.data)
                } else {
                    None
                }
            }
            "ppu" => core.peek_ppu_vram(address as usize),
            "oam" => core.peek_oam(address as usize),
            "palette" => core.peek_palette(address as usize),
            "save" => core
                .peek_cartridge_ram(address as usize)
                .map(|r| r.data),
            _ => None,
        }
    }

    fn write(&mut self, space: &dyn MemorySpace, _address: u32, _value: u8) {
        if self.core_ref().is_none() {
            return;
        }
        match space.id() {
            // Writing via debugger is not supported for Cpu/Ppu/Oam/Palette yet.
            _ => {}
        }
    }
}

// ── NES-specific methods (inherent, accessed via downcast) ──

impl NesDebugger {
    pub fn read_cartridge_ram(&self, address: u16) -> Option<(u8, bool)> {
        let core = self.core_ref()?;
        core.peek_cartridge_ram(address as usize)
            .map(|r| (r.data, r.mask != 0xFF))
    }

    pub fn cpu_registers(&self) -> NesCpuRegisters {
        self.core_ref()
            .map(|core| NesCpuRegisters {
                a: core.cpu_a(),
                x: core.cpu_x(),
                y: core.cpu_y(),
                pc: core.cpu_pc(),
                sp: core.cpu_sp(),
                p: core.cpu_p(),
            })
            .unwrap_or_default()
    }

    pub fn ppu_scanline(&self) -> u16 {
        self.core_ref()
            .map(|core| core.ppu_scanline())
            .unwrap_or(0)
    }

    pub fn ppu_cycle(&self) -> u16 {
        self.core_ref()
            .map(|core| core.ppu_cycle())
            .unwrap_or(0)
    }

    pub fn frame_count(&self) -> u64 {
        self.core_ref()
            .map(|core| core.ppu_frames())
            .unwrap_or(0)
    }
}

//! HLE BIOS bulk-transfer operation (CpuSet/CpuFastSet staging).
//!
//! Lives in its own leaf module so the dependency runs one way:
//! `memory` stores the operation and `bios` constructs it, while neither
//! direction names the other's bus — stepping only sees [`HleBiosBus`].

pub(crate) const CPU_SET_SETUP_CYCLES: u32 = 61;
pub(crate) const CPU_SET_RETURN_CYCLES: u32 = 46;

/// Bus operations needed to step an HLE BIOS transfer. Implemented for
/// `GbaMemoryBus` in `memory.rs`.
pub(crate) trait HleBiosBus {
    fn read8(&mut self, addr: u32) -> u8;
    fn read16(&mut self, addr: u32) -> u16;
    fn read32(&mut self, addr: u32) -> u32;
    fn write_hle_bios16(&mut self, addr: u32, value: u16);
    fn write_hle_bios32(&mut self, addr: u32, value: u32);
}

pub(crate) struct HleBiosOperation {
    source: u32,
    destination: u32,
    remaining: u32,
    fixed: bool,
    width: u8,
    value: u32,
    phase: TransferPhase,
    /// Odd 16-bit sources copy zero-extended odd bytes (not aligned halfwords); one flag covers the transfer.
    /// 32-bit sources align down, except odd SRAM sources (handled below).
    src_odd: bool,
    /// 32-bit CpuSet to an odd SRAM address stores nothing (mgba-suite
    /// "SRAM store swi B 32 (unaligned)" pins residue): the destination
    /// mask would hide the oddness, so it is captured up front.
    /// Other regions keep the masked write (pinned).
    dst_odd_sram_drop: bool,
}

#[derive(Clone, Copy)]
enum TransferPhase {
    Setup(u32),
    Read,
    Write,
    Complete(u32),
}

pub(crate) struct HleStep {
    pub cycles: u32,
    pub complete: bool,
}

impl HleBiosOperation {
    pub(crate) fn cpu_set(source: u32, destination: u32, len_mode: u32) -> Option<Self> {
        let remaining = len_mode & 0x1F_FFFF;
        Self::transfer(source, destination, len_mode, remaining)
    }

    fn transfer(source: u32, destination: u32, len_mode: u32, remaining: u32) -> Option<Self> {
        if remaining == 0 {
            return None;
        }
        let width = if len_mode & (1 << 26) != 0 { 4 } else { 2 };
        // GBATEK CpuSet/CpuFastSet: silently reject when the source start
        // OR end reaches into the BIOS area.
        let end = source as u64 + remaining as u64 * u64::from(width);
        if source < 0x0000_4000 || end - u64::from(width) < 0x0000_4000 {
            return None;
        }
        // mgba-suite "Out-of-bounds load swi B/C" pins zeros: a CpuSet
        // from unmapped memory (below EWRAM, outside BIOS) performs no
        // copy — unlike DMA, which exposes the last bus value. CPU loads
        // from the same addresses still see open bus.
        if (0x0000_4000..0x0200_0000).contains(&source) {
            return None;
        }
        // 16-bit sources keep their odd address (each unit reads the odd
        // byte, see `src_odd`); 32-bit sources align down (pinned),
        // except odd SRAM sources: the 8-bit SRAM bus replicates the
        // odd byte (mgba-suite "SRAM load swi B 32 (unaligned)" pins
        // 0x61616161), which masking would destroy.
        let src_odd = width == 2 && source & 1 != 0;
        let sram_src = (0x0E00_0000..0x1000_0000).contains(&source);
        let keep_src = src_odd || (width == 4 && sram_src && source & 3 != 0);
        Some(Self {
            source: if keep_src {
                source
            } else {
                source & !(u32::from(width) - 1)
            },
            destination: destination & !(u32::from(width) - 1),
            remaining,
            fixed: len_mode & (1 << 24) != 0,
            width,
            value: 0,
            phase: TransferPhase::Setup(CPU_SET_SETUP_CYCLES),
            src_odd,
            dst_odd_sram_drop: width == 4
                && destination & 3 != 0
                && (0x0E00_0000..0x1000_0000).contains(&destination),
        })
    }

    pub(crate) fn step(&mut self, bus: &mut impl HleBiosBus) -> HleStep {
        match self.phase {
            TransferPhase::Setup(remaining) => {
                self.phase = if remaining == 1 {
                    TransferPhase::Read
                } else {
                    TransferPhase::Setup(remaining - 1)
                };
                HleStep {
                    cycles: 1,
                    complete: false,
                }
            }
            TransferPhase::Read => {
                self.value = if self.width == 4 {
                    bus.read32(self.source)
                } else if self.src_odd {
                    u32::from(bus.read8(self.source))
                } else {
                    u32::from(bus.read16(self.source))
                };
                if !self.fixed {
                    self.source = self.source.wrapping_add(u32::from(self.width));
                }
                self.phase = TransferPhase::Write;
                HleStep {
                    cycles: 1,
                    complete: false,
                }
            }
            TransferPhase::Write => {
                // mgba-suite "SRAM store swi B 32 (unaligned)" pins
                // residue: a 32-bit CpuSet to an odd SRAM address stores
                // nothing (the 8-bit SRAM bus drops unaligned word
                // stores); other regions keep the masked write (pinned).
                let sram_odd_drop = self.dst_odd_sram_drop;
                if !sram_odd_drop {
                    if self.width == 4 {
                        bus.write_hle_bios32(self.destination, self.value);
                    } else {
                        bus.write_hle_bios16(self.destination, self.value as u16);
                    }
                }
                self.destination = self.destination.wrapping_add(u32::from(self.width));
                self.remaining -= 1;
                self.phase = if self.remaining == 0 {
                    TransferPhase::Complete(CPU_SET_RETURN_CYCLES)
                } else {
                    TransferPhase::Read
                };
                HleStep {
                    cycles: 1,
                    complete: false,
                }
            }
            TransferPhase::Complete(remaining) => {
                self.phase = TransferPhase::Complete(remaining.saturating_sub(1));
                HleStep {
                    cycles: 1,
                    complete: remaining == 1,
                }
            }
        }
    }
}

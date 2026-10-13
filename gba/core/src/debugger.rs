//! GBA debugger address space.
//!
//! Deliberately a single space: regions are already distinguished by
//! address, so extra ids would buy nothing — the reader dispatches on
//! the address internally. The one trade-off is typo detection: gap
//! addresses resolve inside the space and read as `None` (`OpenBus`)
//! instead of failing loudly as `Unmapped`. Never assert `open_bus`
//! on a gap address.
//!
//! Reader contract (for the next step; all verified against
//! `memory/mod.rs` decode):
//!
//! - New `&self` peek only: the existing `read8/16/32` are `&mut` and
//!   charge wait cycles plus N/S-tracker progress. The region readers
//!   underneath (`read_ewram/iwram/palette/vram/oam/rom/sram`,
//!   `read_bios`) are already `&self` and reusable as-is.
//! - BIOS guard reproduced without mutation: protected reads serve the
//!   `bios_prefetch` latch (`&self` field read); only unprotected
//!   reads touch `read_bios`.
//! - OAM without the `cpu_bus` latch store that `read_mapped` performs.
//! - `0x0D` window: EEPROM cartridges serve `eeprom_peek_bit` (existing
//!   `&self` peek); plain ROMs mirror WS2 through `read_rom`. GPIO
//!   overlays apply in the same order as `read_rom`.
//! - SRAM answers only with a battery-backed chip: EEPROM cartridges
//!   and no-cartridge reads are `None` (honest open bus), never a
//!   fabricated `0xFF`.
//! - I/O (`0x04000000..=0x04000803`) serves the audited pure subset —
//!   PPU/APU/timers/key/IRQ fields, DMA CNT, constant holes — and
//!   returns `None` for the stateful SIO block (UART latch clear, FIFO
//!   pop), write-only registers, and gap lanes. Timer reads reuse the
//!   stamped cycle (no re-stamp, no mutation). Unmapped I/O is dynamic
//!   prefetch-latch open bus on hardware; the debugger does not
//!   reproduce it.
//! - Widths 2/4 compose physical bytes little-endian. CPU aligned-ROR
//!   semantics deliberately do not apply: the debugger shows physical
//!   contents.

use std::cell::OnceCell;

use nerust_core_traits::debugger::{
    DebugImage, Debugger, DisasmLine, ImageFormat, SpaceAccess, SpaceId, SpaceInfo, SpaceTable,
    rgb555_to_rgb888,
};

use crate::system::GbaSystem;

/// Whole decoded bus (`0x00000000..=0x0FFFFFFF`). The only GBA space.
/// Above `0x0FFFFFFF` stays `Unmapped` (loud manifest error).
pub const SPACE_MEMORY: SpaceId = SpaceId(0);

static GBA_SPACES: [SpaceInfo; 1] = [SpaceInfo {
    id: SPACE_MEMORY,
    key: "memory",
    name: "Memory",
    address_bits: 32,
    range: 0x00000000..=0x0FFFFFFF,
    access: SpaceAccess::ReadWrite,
}];

/// Validated GBA memory-space table.
///
/// BIOS and ROM are not pokeable: stores there vanish on hardware
/// (ROM also overlays GPIO). The windows live in the table so the
/// write gate and the UI read the same data.
pub(crate) static GBA_SPACE_TABLE: SpaceTable = SpaceTable::build_with_read_only(
    &GBA_SPACES,
    &[
        (SPACE_MEMORY, 0x00000000, 0x00003FFF),
        (SPACE_MEMORY, 0x08000000, 0x0DFFFFFF),
    ],
);

/// GBA observer.
///
/// Built per use (built, read, dropped); `registers()` borrows a buffer
/// filled once in `new`.
///
/// Register names are plain `r0`-`r15` plus `cpsr`, in that order. This
/// deliberately bends the kernel's ascending-order clause at `r10`
/// (`"r10" < "r2"` lexically): conventional names won over padding.
/// Presentation must not re-sort.
///
/// `r()` ORs the user bank during the post-LDM conflict window: the
/// list reports what the register reads as, transient included —
/// deterministic at a fixed pause point all the same.
pub struct GbaDebugger<'a> {
    system: &'a GbaSystem,
    /// Lazily filled on first `registers()`: screen/memory-only
    /// asserts never pay for register collection.
    regs: OnceCell<Vec<(&'static str, u64)>>,
}

impl<'a> GbaDebugger<'a> {
    pub fn new(system: &'a GbaSystem) -> Self {
        Self {
            system,
            regs: OnceCell::new(),
        }
    }

    fn snapshot_registers(&self) -> Vec<(&'static str, u64)> {
        let regs = self.system.cpu.registers();
        let mut out = Vec::with_capacity(18);
        out.push(("cpsr", u64::from(regs.cpsr())));
        // Leak-free static names: the index selects a literal, so no
        // allocation and no loss of the ascending intent.
        const NAMES: [&str; 16] = [
            "r0", "r1", "r2", "r3", "r4", "r5", "r6", "r7", "r8", "r9", "r10", "r11", "r12", "r13",
            "r14", "r15",
        ];
        for (i, name) in NAMES.iter().enumerate() {
            out.push((*name, u64::from(regs.r(i))));
        }
        out
    }

    /// One Thumb row (or raw-byte fallback rows): pushes into `out`,
    /// returns the cursor advance. A Thumb BL pair decodes as one
    /// 4-byte row; lone halves stay `.HWORD` via the shared decoder.
    fn thumb_row(
        cursor: u32,
        raw: [u8; 4],
        have: u32,
        is_pc: bool,
        out: &mut Vec<DisasmLine>,
    ) -> u32 {
        use crate::disasm_gba::decode_thumb;
        if have < 2 {
            out.push(raw_db(cursor, raw[0], is_pc));
            return 1;
        }
        let word = u16::from_le_bytes([raw[0], raw[1]]);
        // BL pair: verify the second half before combining.
        if (0xF000..0xF800).contains(&word) && have == 4 {
            let next = u16::from_le_bytes([raw[2], raw[3]]);
            if next >= 0xF800 {
                let combined = (u32::from(word & 0x3FF) << 12) | (u32::from(next & 0x7FF) << 1);
                let offset = ((combined << 9) as i32) >> 9;
                let dest = cursor.wrapping_add(4).wrapping_add(offset as u32);
                out.push(DisasmLine {
                    addr: cursor,
                    bytes: raw,
                    len: 4,
                    text: format!("BL ${dest:08X}"),
                    is_pc,
                    target: Some(dest),
                });
                return 4;
            }
        }
        let decoded = decode_thumb(cursor, word);
        out.push(DisasmLine {
            addr: cursor,
            bytes: [raw[0], raw[1], 0, 0],
            len: 2,
            text: decoded.text,
            is_pc,
            target: decoded.target,
        });
        2
    }

    /// One ARM row (or per-byte fallback rows): pushes into `out`,
    /// returns the cursor advance. Fallback rows advance the address
    /// per byte, mirroring the inline loop this replaces.
    fn arm_row(
        cursor: u32,
        raw: [u8; 4],
        have: u32,
        pc: Option<u32>,
        out: &mut Vec<DisasmLine>,
    ) -> u32 {
        use crate::disasm_gba::decode_arm;
        let is_pc = pc == Some(cursor);
        if have < 4 {
            for (addr, b) in (cursor..).zip(raw.iter().take(have as usize)) {
                out.push(raw_db(addr, *b, pc == Some(addr)));
            }
            return have;
        }
        let word = u32::from_le_bytes(raw);
        let decoded = decode_arm(cursor, word);
        out.push(DisasmLine {
            addr: cursor,
            bytes: raw,
            len: 4,
            text: decoded.text,
            is_pc,
            target: decoded.target,
        });
        4
    }
}

impl Debugger for GbaDebugger<'_> {
    fn spaces(&self) -> &[SpaceInfo] {
        &GBA_SPACES
    }

    fn space_containing(&self, addr: u32) -> Option<SpaceId> {
        GBA_SPACES
            .iter()
            .find(|s| s.range.contains(&addr))
            .map(|s| s.id)
    }

    fn read(&self, space: SpaceId, addr: u32, width: u8) -> Option<u64> {
        // Single gate: unknown SpaceId, out-of-range, range-crossing,
        // and bad widths (including 0) all become None here.
        if !GBA_SPACE_TABLE.covers(space, addr, width) {
            return None;
        }
        // Little-endian composition of physical bytes. CPU aligned-ROR
        // semantics do not apply: the debugger shows physical contents.
        let mut value = 0u64;
        for i in 0..width as u32 {
            value |= u64::from(self.system.bus.peek_byte(addr + i)?) << (8 * i);
        }
        Some(value)
    }

    fn registers(&self) -> &[(&'static str, u64)] {
        self.regs.get_or_init(|| self.snapshot_registers())
    }

    fn images(&self) -> Vec<DebugImage> {
        // 4bpp tiles from the first two charblock pairs: 1024 tiles
        // each in a 32x32 grid (256x256). Bytes unreadable through
        // the peek path decode as blank; palette is BG 0.
        const TILES: u32 = 1024;
        const COLS: u32 = 32;
        let mut palette = Vec::with_capacity(16);
        for i in 0..16u32 {
            let raw = self.read(SPACE_MEMORY, 0x05000000 + 2 * i, 2).unwrap_or(0);
            palette.push(rgb555_to_rgb888(raw as u16));
        }
        [("tiles-low", 0x06000000), ("tiles-high", 0x06008000)]
            .into_iter()
            .map(|(id, base)| {
                let mut pixels = vec![0u8; 256 * 256];
                for tile in 0..TILES {
                    let tx = (tile % COLS) * 8;
                    let ty = (tile / COLS) * 8;
                    for row in 0..8u32 {
                        for col in 0..8u32 {
                            let addr = base + tile * 32 + row * 4 + col / 2;
                            let byte = self.read(SPACE_MEMORY, addr, 1).unwrap_or(0) as u8;
                            let px = if col & 1 == 0 { byte & 0xF } else { byte >> 4 };
                            pixels[(ty + row) as usize * 256 + (tx + col) as usize] = px;
                        }
                    }
                }
                DebugImage {
                    id,
                    label_id: if base == 0x06000000 {
                        "Tiles low"
                    } else {
                        "Tiles high"
                    },
                    width: 256,
                    height: 256,
                    format: ImageFormat::Indexed4bpp,
                    palette: palette.clone(),
                    pixels,
                }
            })
            .collect()
    }

    /// Anchor for disassembly follow mode. The architectural `r15`
    /// value (Thumb bit cleared by `pc()`); presentation never scans
    /// the register list for it.
    fn program_counter(&self) -> Option<u32> {
        Some(self.system.cpu.registers().pc())
    }

    /// Disassemble `count` rows from `addr` in the CPSR-selected set.
    /// Short reads at the range end decode as single raw bytes so a
    /// truncated tail never panics and never fabricates instructions.
    /// A Thumb BL pair (first half `0xF000-0xF7FF`, second
    /// `0xF800-0xFFFF`) decodes as one 4-byte row; lone halves stay
    /// `.HWORD`.
    fn disassemble(&self, addr: u32, count: u16) -> Vec<DisasmLine> {
        let pc = self.program_counter();
        let thumb = self.system.cpu.registers().cpsr_t();
        let mut out = Vec::new();
        let mut cursor = addr;
        for _ in 0..count {
            if cursor > 0x0FFFFFFF {
                break;
            }
            // Up to 4 physical bytes; fewer means the range end.
            let mut raw = [0u8; 4];
            let mut have = 0u32;
            while have < 4 {
                match self.read(SPACE_MEMORY, cursor + have, 1) {
                    Some(v) => {
                        raw[have as usize] = v as u8;
                        have += 1;
                    }
                    None => break,
                }
            }
            if have == 0 {
                break;
            }
            let is_pc = pc == Some(cursor);
            let advance = if thumb {
                Self::thumb_row(cursor, raw, have, is_pc, &mut out)
            } else {
                Self::arm_row(cursor, raw, have, pc, &mut out)
            };
            cursor += advance;
        }
        out
    }
}

/// Single raw byte row for truncated tails.
fn raw_db(addr: u32, byte: u8, is_pc: bool) -> DisasmLine {
    DisasmLine {
        addr,
        bytes: [byte, 0, 0, 0],
        len: 1,
        text: format!(".DB ${byte:02X}"),
        is_pc,
        target: None,
    }
}

#[cfg(test)]
mod tests {
    use nerust_core_traits::debugger::{Debugger, validate_spaces};

    use super::*;

    #[test]
    fn gba_spaces_validate() {
        GBA_SPACE_TABLE.validate().expect("GBA spaces valid");
        assert_eq!(validate_spaces(&GBA_SPACES), Ok(()));
    }

    #[test]
    fn gba_space_is_single_decoded_bus_range() {
        assert_eq!(GBA_SPACES.len(), 1);
        let space = &GBA_SPACES[0];
        assert_eq!(space.id, SPACE_MEMORY);
        assert_eq!(*space.range.start(), 0x00000000);
        assert_eq!(*space.range.end(), 0x0FFFFFFF);
        // covers() is the read/write gate: every decoded region
        // resolves here; beyond decode is rejected.
        assert!(GBA_SPACE_TABLE.covers(SPACE_MEMORY, 0x00000000, 4));
        assert!(GBA_SPACE_TABLE.covers(SPACE_MEMORY, 0x02000000, 1));
        assert!(GBA_SPACE_TABLE.covers(SPACE_MEMORY, 0x04000208, 2));
        assert!(GBA_SPACE_TABLE.covers(SPACE_MEMORY, 0x08000000, 4));
        assert!(GBA_SPACE_TABLE.covers(SPACE_MEMORY, 0x0D000000, 1));
        assert!(GBA_SPACE_TABLE.covers(SPACE_MEMORY, 0x0E000000, 1));
        assert!(GBA_SPACE_TABLE.covers(SPACE_MEMORY, 0x0FFFFFFF, 1));
        assert!(!GBA_SPACE_TABLE.covers(SPACE_MEMORY, 0x0FFFFFFF, 2));
        assert!(!GBA_SPACE_TABLE.covers(SPACE_MEMORY, 0x10000000, 1));
        assert!(!GBA_SPACE_TABLE.covers(SPACE_MEMORY, 0xFFFFFFFF, 4));
        assert!(!GBA_SPACE_TABLE.covers(SpaceId(1), 0x02000000, 1));
        assert!(!GBA_SPACE_TABLE.covers(SPACE_MEMORY, 0x02000000, 0));
    }

    fn rom() -> Vec<u8> {
        let mut rom = vec![0; 0x4000];
        // Header, logo, and complement are filled by the finalizer.
        crate::cartridge::header::finalize_test_gba_rom(&mut rom);
        rom
    }

    fn live_system() -> GbaSystem {
        // The debugger borrows the live system; the test ROM builds one
        // the same way load does.
        GbaSystem::from_test_rom(rom()).expect("test ROM builds a system")
    }

    #[test]
    fn gba_program_counter_matches_r15_masked() {
        let system = live_system();
        let debugger = GbaDebugger::new(&system);
        let r15 = debugger
            .registers()
            .iter()
            .find(|(name, _)| *name == "r15")
            .map(|(_, value)| *value as u32);
        assert_eq!(debugger.program_counter(), r15.map(|pc| pc & !1));
    }

    #[test]
    fn gba_images_tile_blocks() {
        let system = live_system();
        let debugger = GbaDebugger::new(&system);
        let images = debugger.images();
        assert_eq!(images.len(), 2);
        for image in &images {
            assert_eq!((image.width, image.height), (256, 256));
            assert_eq!(image.format, ImageFormat::Indexed4bpp);
            assert_eq!(image.palette.len(), 16);
            assert_eq!(image.pixels.len(), 256 * 256);
            assert!(image.pixels.iter().all(|&p| p < 16));
        }
        assert_eq!(images[0].id, "tiles-low");
        assert_eq!(images[1].id, "tiles-high");
    }

    #[test]
    fn gba_disassemble_shape_and_tail() {
        let system = live_system();
        let debugger = GbaDebugger::new(&system);
        // Uniform rows: one mode per pause point, stride matches len.
        let rows = debugger.disassemble(0x08000000, 8);
        assert_eq!(rows.len(), 8);
        let len = rows[0].len;
        assert!(len == 2 || len == 4);
        for (i, row) in rows.iter().enumerate() {
            assert_eq!(row.len, len);
            assert_eq!(row.addr, 0x08000000 + (i as u32) * u32::from(len));
        }
        // Truncated tail at the range end: raw bytes, no panic.
        let tail = debugger.disassemble(0x0FFFFFFF, 4);
        assert!(!tail.is_empty());
        assert!(tail.iter().all(|row| row.len == 1));
        assert!(tail.iter().all(|row| row.target.is_none()));
        // Past the range: empty, never fabricated.
        assert!(debugger.disassemble(0x10000000, 4).is_empty());
    }

    #[test]
    fn gba_debugger_can_be_boxed() {
        let system = live_system();
        let boxed: Box<dyn Debugger + '_> = Box::new(GbaDebugger::new(&system));
        assert_eq!(boxed.spaces().len(), 1);
        assert_eq!(boxed.space_containing(0x02000000), Some(SPACE_MEMORY));
        assert_eq!(boxed.space_containing(0x08000000), Some(SPACE_MEMORY));
        assert_eq!(boxed.space_containing(0x10000000), None);
        // Registers: cpsr then r0-r15 in order (plain names by design).
        let names: Vec<&str> = boxed.registers().iter().map(|(n, _)| *n).collect();
        let mut expected = vec!["cpsr"];
        expected.extend(
            [
                "r0", "r1", "r2", "r3", "r4", "r5", "r6", "r7", "r8", "r9", "r10", "r11", "r12",
                "r13", "r14", "r15",
            ]
            .iter(),
        );
        assert_eq!(names, expected);
        // Live read path: the ROM logo byte is observable through the
        // game-pak window (the GBA logo starts 0x24, not the GB logo).
        assert_eq!(boxed.read(SPACE_MEMORY, 0x08000004, 1), Some(0x24));
        // Fresh EWRAM is readable.
        assert!(boxed.read(SPACE_MEMORY, 0x02000000, 1).is_some());
        // Little-endian composition.
        let lo = boxed.read(SPACE_MEMORY, 0x08000004, 1).unwrap();
        let wide = boxed.read(SPACE_MEMORY, 0x08000004, 2).unwrap();
        assert_eq!(wide & 0xFF, lo);
        // Unmapped and bad inputs collapse to None.
        assert_eq!(boxed.read(SPACE_MEMORY, 0x10000000, 1), None);
        assert_eq!(boxed.read(SPACE_MEMORY, 0x0FFFFFFF, 2), None);
        assert_eq!(boxed.read(SPACE_MEMORY, 0x02000000, 0), None);
        assert_eq!(boxed.read(SPACE_MEMORY, 0x02000000, 3), None);
        assert_eq!(boxed.read(SpaceId(9), 0x02000000, 1), None);
    }

    #[test]
    fn gba_debugger_reads_wide_and_rereads_registers() {
        let system = live_system();
        let debugger = GbaDebugger::new(&system);
        // Width-4 composition over the logo bytes.
        let wide = debugger
            .read(SPACE_MEMORY, 0x08000004, 4)
            .expect("wide read");
        assert_eq!(wide & 0xFF, 0x24);
        // Register reads are cached but stable across calls.
        assert_eq!(debugger.registers().len(), 17);
        assert_eq!(debugger.registers().len(), 17);
        // Misses and range crossings collapse to None.
        assert_eq!(debugger.space_containing(0xFFFF_FFFF), None);
        assert_eq!(debugger.read(SPACE_MEMORY, 0x0FFF_FFFE, 4), None);
    }
}

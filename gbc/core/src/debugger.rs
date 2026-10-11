//! GBC debugger address space.
//!
//! Deliberately a single space: regions are already distinguished by
//! address, so extra ids would buy nothing — the reader dispatches on
//! the address internally. The one trade-off is typo detection: gap
//! addresses resolve inside the space and read as `None` (`OpenBus`)
//! instead of failing loudly as `Unmapped`. Never assert `open_bus`
//! on a gap address.
//!
//! Reader contract (for the next step; all verified against
//! `memory.rs` decode):
//!
//! - Echo `0xE000..=0xFDFF` normalizes to `0xC000..=0xDDFF` (`-0x2000`).
//! - `0xFFFF` serves the IE register, not the HRAM array.
//! - The unusable gap `0xFEA0..=0xFEFF` reads `0x00` on hardware;
//!   return `Some(0x00)` (mapped), not `None`.
//! - `0xA000..=0xBFFF` reads `0xFF` when RAM is disabled or absent
//!   (the MBC exposes no driven/floating mask): byte-exact, but
//!   `open_bus` assertions are meaningless there.
//! - CGB-only registers return their fixed DMG values; VRAM reads go
//!   through `debug_read_vram` (no mode locks); OAM is readable —
//!   frame steps always park in VBlank, so the value is deterministic.
//! - All sub-reads are `&self` with no mutation (audited: APU, PPU,
//!   serial, timer, interrupts, joypad).

use std::cell::OnceCell;

use nerust_core_traits::debugger::{
    DebugImage, Debugger, DisasmLine, ImageFormat, SpaceAccess, SpaceId, SpaceInfo, SpaceTable,
};

use crate::system::GbcSystem;

/// Whole 16-bit bus (`0x0000..=0xFFFF`). The only GBC space.
pub const SPACE_MEMORY: SpaceId = SpaceId(0);

static GBC_SPACES: [SpaceInfo; 1] = [SpaceInfo {
    id: SPACE_MEMORY,
    key: "memory",
    name: "Memory",
    address_bits: 16,
    range: 0x0000..=0xFFFF,
    access: SpaceAccess::ReadWrite,
}];

/// Validated GBC memory-space table.
///
/// ROM is not pokeable: a CPU store there drives MBC commands. The
/// window lives in the table so the write gate and the UI read the
/// same data.
pub(crate) static GBC_SPACE_TABLE: SpaceTable =
    SpaceTable::build_with_read_only(&GBC_SPACES, &[(SPACE_MEMORY, 0x0000, 0x7FFF)]);

/// GBC observer.
///
/// Built per use (built, read, dropped); `registers()` borrows a buffer
/// filled lazily on first use. Every decoded address is driven —
/// there are no floating reads — so `read` is `Some` for the whole
/// range and `None` only outside it. Writes land where hardware is
/// writable (VRAM/OAM bypass the PPU mode locks); ROM refuses as
/// `ReadOnlySpace` instead of driving MBC commands.
pub struct GbcDebugger<'a> {
    system: &'a GbcSystem,
    /// Lazily filled on first `registers()`: screen/memory-only
    /// asserts never pay for register collection.
    regs: OnceCell<Vec<(&'static str, u64)>>,
}

impl<'a> GbcDebugger<'a> {
    pub fn new(system: &'a GbcSystem) -> Self {
        Self {
            system,
            regs: OnceCell::new(),
        }
    }

    fn snapshot_registers(&self) -> Vec<(&'static str, u64)> {
        let regs = self.system.cpu.registers();
        Vec::from([
            ("a", u64::from(regs.a())),
            ("b", u64::from(regs.b())),
            ("c", u64::from(regs.c())),
            ("d", u64::from(regs.d())),
            ("e", u64::from(regs.e())),
            ("f", u64::from(regs.f())),
            ("h", u64::from(regs.h())),
            ("l", u64::from(regs.l())),
            ("pc", u64::from(regs.pc())),
            ("sp", u64::from(regs.sp())),
        ])
    }

    fn read_byte(&self, addr: u32) -> Option<u64> {
        Some(u64::from(self.system.bus.debug_read(addr as u16)))
    }
}

impl Debugger for GbcDebugger<'_> {
    fn spaces(&self) -> &[SpaceInfo] {
        &GBC_SPACES
    }

    fn space_containing(&self, addr: u32) -> Option<SpaceId> {
        GBC_SPACES
            .iter()
            .find(|s| s.range.contains(&addr))
            .map(|s| s.id)
    }

    fn read(&self, space: SpaceId, addr: u32, width: u8) -> Option<u64> {
        // Single gate: unknown SpaceId, out-of-range, range-crossing,
        // and bad widths (including 0) all become None here. The cast
        // below is safe: covers() pins addr + width - 1 <= 0xFFFF.
        if !GBC_SPACE_TABLE.covers(space, addr, width) {
            return None;
        }
        // Little-endian composition of physical bytes.
        let mut value = 0u64;
        for i in 0..width as u32 {
            value |= self.read_byte(addr + i)? << (8 * i);
        }
        Some(value)
    }

    fn registers(&self) -> &[(&'static str, u64)] {
        self.regs.get_or_init(|| self.snapshot_registers())
    }

    fn program_counter(&self) -> Option<u32> {
        self.registers()
            .iter()
            .find(|(name, _)| *name == "pc")
            .map(|(_, value)| *value as u32)
    }

    fn disassemble(&self, addr: u32, count: u16) -> Vec<DisasmLine> {
        use crate::disasm_gb::{decode, instr_len};
        let pc = self.program_counter();
        let mut out = Vec::new();
        let mut cursor = addr;
        for _ in 0..count {
            if cursor > 0xFFFF {
                break;
            }
            let op = match self.read(SPACE_MEMORY, cursor, 1) {
                Some(op) => op as u8,
                None => break,
            };
            // Up to 2 operand bytes; fewer means the range end.
            let mut raw = [op, 0, 0];
            let mut have = 1u32;
            while have < 3 {
                match self.read(SPACE_MEMORY, cursor + have, 1) {
                    Some(v) => {
                        raw[have as usize] = v as u8;
                        have += 1;
                    }
                    None => break,
                }
            }
            let want = u32::from(instr_len(op));
            if have < want {
                // Truncated tail: raw bytes, never fabricated.
                for b in raw.iter().take(have as usize) {
                    out.push(raw_db(cursor, *b, pc == Some(cursor)));
                    cursor += 1;
                }
                continue;
            }
            let is_pc = pc == Some(cursor);
            if op == 0xCB {
                let decoded = decode(cursor, op, raw[1], 0);
                out.push(DisasmLine {
                    addr: cursor,
                    bytes: [op, raw[1], 0, 0],
                    len: 2,
                    text: decoded.text,
                    is_pc,
                    target: None,
                });
                cursor += 2;
                continue;
            }
            let decoded = decode(cursor, op, raw[1], raw[2]);
            let mut bytes = [0u8; 4];
            bytes[..decoded.len as usize].copy_from_slice(&raw[..decoded.len as usize]);
            out.push(DisasmLine {
                addr: cursor,
                bytes,
                len: decoded.len,
                text: decoded.text,
                is_pc,
                target: decoded.target,
            });
            cursor += u32::from(decoded.len);
        }
        out
    }

    fn images(&self) -> Vec<DebugImage> {
        // One image per VRAM bank: 384 tiles in a 16x24 grid
        // (128x192). Bank reads bypass VBK state; palette is BG 0.
        const TILES: u16 = 384;
        const COLS: u16 = 16;
        let ppu = self.system.bus.debug_ppu();
        let palette = ppu.debug_bg_palette();
        [("tiles-bank0", 0), ("tiles-bank1", 1)]
            .into_iter()
            .map(|(id, bank)| {
                let mut pixels = vec![0u8; 128 * 192];
                for tile in 0..TILES {
                    let tx = (tile % COLS) * 8;
                    let ty = (tile / COLS) * 8;
                    for row in 0..8u16 {
                        let base = tile * 16 + row * 2;
                        let lo = ppu.debug_read_vram_bank(bank, base);
                        let hi = ppu.debug_read_vram_bank(bank, base + 1);
                        for col in 0..8u16 {
                            let bit = 7 - col;
                            let px = ((hi >> bit) & 1) << 1 | ((lo >> bit) & 1);
                            pixels[(ty + row) as usize * 128 + (tx + col) as usize] = px;
                        }
                    }
                }
                DebugImage {
                    id,
                    label_id: if bank == 0 {
                        "Tiles bank 0"
                    } else {
                        "Tiles bank 1"
                    },
                    width: 128,
                    height: 192,
                    format: ImageFormat::Indexed2bpp,
                    palette: palette.to_vec(),
                    pixels,
                }
            })
            .collect()
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
    use nerust_core_traits::debugger::validate_spaces;

    use super::*;

    #[test]
    fn gbc_spaces_validate() {
        GBC_SPACE_TABLE.validate().expect("GBC spaces valid");
        assert_eq!(validate_spaces(&GBC_SPACES), Ok(()));
    }

    #[test]
    fn gbc_space_is_single_full_bus_range() {
        assert_eq!(GBC_SPACES.len(), 1);
        let space = &GBC_SPACES[0];
        assert_eq!(space.id, SPACE_MEMORY);
        assert_eq!(*space.range.start(), 0x0000);
        assert_eq!(*space.range.end(), 0xFFFF);
        // covers() is the read/write gate: the full bus resolves here;
        // anything wider is rejected.
        assert!(GBC_SPACE_TABLE.covers(SPACE_MEMORY, 0x0000, 1));
        assert!(GBC_SPACE_TABLE.covers(SPACE_MEMORY, 0xC000, 2));
        assert!(GBC_SPACE_TABLE.covers(SPACE_MEMORY, 0xE123, 1));
        assert!(GBC_SPACE_TABLE.covers(SPACE_MEMORY, 0xFEA0, 1));
        assert!(GBC_SPACE_TABLE.covers(SPACE_MEMORY, 0xFFFF, 1));
        assert!(GBC_SPACE_TABLE.covers(SPACE_MEMORY, 0xFFFE, 2));
        assert!(!GBC_SPACE_TABLE.covers(SPACE_MEMORY, 0xFFFF, 2));
        assert!(!GBC_SPACE_TABLE.covers(SPACE_MEMORY, 0xFFFF, 4));
        assert!(!GBC_SPACE_TABLE.covers(SPACE_MEMORY, 0x10000, 1));
        assert!(!GBC_SPACE_TABLE.covers(SpaceId(1), 0xC000, 1));
        assert!(!GBC_SPACE_TABLE.covers(SPACE_MEMORY, 0xC000, 0));
    }

    fn rom() -> Vec<u8> {
        let mut rom = vec![0; 0x8000];
        rom[0x0100] = 0x18; // JR -2
        rom[0x0101] = 0xFE;
        rom[0x0143] = 0x80;
        rom[0x0147] = 0;
        rom[0x0148] = 0;
        rom[0x0149] = 0;
        crate::cartridge_header::finalize_test_rom(&mut rom);
        rom
    }

    fn live_system() -> GbcSystem {
        // The debugger borrows the live system; tests build one the
        // same way load does.
        let rom = rom();
        let descriptor = crate::cartridge_descriptor::detect_cartridge(&rom).expect("header");
        GbcSystem::from_descriptor(
            crate::core_options::GbcCoreOptions::default().hardware_model,
            rom,
            &descriptor,
        )
        .expect("system builds")
    }

    #[test]
    fn gbc_debugger_can_be_boxed() {
        let system = live_system();
        let boxed: Box<dyn Debugger + '_> = Box::new(GbcDebugger::new(&system));
        assert_eq!(boxed.spaces().len(), 1);
        assert_eq!(boxed.space_containing(0xC000), Some(SPACE_MEMORY));
        assert_eq!(boxed.space_containing(0xFFFF), Some(SPACE_MEMORY));
        assert_eq!(boxed.space_containing(0x10000), None);
        // Registers: 10 entries in ascending name order (kernel contract).
        let names: Vec<&str> = boxed.registers().iter().map(|(n, _)| *n).collect();
        assert_eq!(
            names,
            vec!["a", "b", "c", "d", "e", "f", "h", "l", "pc", "sp"]
        );
        // Live read path: fresh WRAM is readable, and the echo
        // mirror observes the same byte.
        assert!(boxed.read(SPACE_MEMORY, 0xC000, 1).is_some());
        assert_eq!(
            boxed.read(SPACE_MEMORY, 0xC000, 1),
            boxed.read(SPACE_MEMORY, 0xE000, 1)
        );
        // Little-endian composition.
        let lo = boxed.read(SPACE_MEMORY, 0xC000, 1).unwrap();
        let wide = boxed.read(SPACE_MEMORY, 0xC000, 2).unwrap();
        assert_eq!(wide & 0xFF, lo);
        // Unusable gap reads its hardware value, not None.
        assert_eq!(boxed.read(SPACE_MEMORY, 0xFEA0, 1), Some(0x00));
        // Rejected inputs collapse to None.
        assert_eq!(boxed.read(SPACE_MEMORY, 0xFFFF, 2), None);
        assert_eq!(boxed.read(SPACE_MEMORY, 0x10000, 1), None);
        assert_eq!(boxed.read(SPACE_MEMORY, 0xC000, 0), None);
        assert_eq!(boxed.read(SPACE_MEMORY, 0xC000, 3), None);
        assert_eq!(boxed.read(SpaceId(9), 0xC000, 1), None);
    }

    #[test]
    fn gbc_debugger_images_tile_banks() {
        let system = live_system();
        let debugger = GbcDebugger::new(&system);
        let images = debugger.images();
        assert_eq!(images.len(), 2);
        for image in &images {
            assert_eq!((image.width, image.height), (128, 192));
            assert_eq!(image.format, ImageFormat::Indexed2bpp);
            assert_eq!(image.palette.len(), 4);
            assert_eq!(image.pixels.len(), 128 * 192);
            assert!(image.pixels.iter().all(|&p| p < 4));
        }
        assert_eq!(images[0].id, "tiles-bank0");
        assert_eq!(images[1].id, "tiles-bank1");
    }
}

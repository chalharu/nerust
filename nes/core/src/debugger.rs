//! NES debugger: read-only observation plus controlled WRAM edits.
//!
//! Design: `debugger-trait-design.md` v7.0 §6.5. The observer is per-use
//! (built, read, dropped); `registers()` borrows a buffer filled once in
//! `new`, since `RefCell` cannot hand out borrows (E0515).

use std::cell::OnceCell;

use nerust_core_traits::debugger::{
    CellValue, ColumnKind, DebugImage, DebugPanel, Debugger, DisasmLine, PanelColumn, PanelRow,
    SpaceAccess, SpaceId, SpaceInfo, SpaceTable,
};

use crate::Core;

/// Work RAM with CPU mirrors ($0000-$07FF mirrored to $1FFF).
/// Matches `peek_work_ram` semantics exactly.
pub const SPACE_WORK_RAM: SpaceId = SpaceId(0);
/// PPU nametables and palette ($2000-$3EFF, $3F00-$3FFF).
/// CHR ($0000-$1FFF) is cartridge-dependent and intentionally excluded:
/// it has no stable range in this table.
pub const SPACE_PPU_VRAM: SpaceId = SpaceId(1);
/// Cartridge PRG RAM ($6000-$7FFF). Reads go through the bus-state
/// preserving peek: fully-driven bytes read as values, floating bytes
/// read as open (None). Absent from older tables — resolvers must use
/// the table itself, never positions.
pub const SPACE_CARTRIDGE_RAM: SpaceId = SpaceId(2);
/// PRG ROM window ($8000-$FFFF, ReadOnly) backing disassembly and the
/// program view.
pub const SPACE_PRG_ROM: SpaceId = SpaceId(3);

static NES_SPACES: [SpaceInfo; 4] = [
    SpaceInfo {
        id: SPACE_WORK_RAM,
        key: "wram",
        name: "WRAM",
        address_bits: 13,
        range: 0x0000..=0x1FFF,
        access: SpaceAccess::ReadWrite,
    },
    SpaceInfo {
        id: SPACE_PPU_VRAM,
        key: "ppu_vram",
        name: "PPU VRAM",
        address_bits: 14,
        range: 0x2000..=0x3FFF,
        // No VRAM poke exists yet, so edits are refused for now.
        access: SpaceAccess::ReadOnly,
    },
    SpaceInfo {
        id: SPACE_CARTRIDGE_RAM,
        key: "cartridge_ram",
        name: "Cartridge RAM",
        address_bits: 13,
        range: 0x6000..=0x7FFF,
        // Pokes target WRAM only; cartridge bytes are read-only here.
        access: SpaceAccess::ReadOnly,
    },
    SpaceInfo {
        id: SPACE_PRG_ROM,
        key: "prg_rom",
        name: "PRG ROM",
        address_bits: 16,
        range: 0x8000..=0xFFFF,
        access: SpaceAccess::ReadOnly,
    },
];

/// Validated NES memory-space table.
pub(crate) static NES_SPACE_TABLE: SpaceTable = SpaceTable::build(&NES_SPACES);

/// Read-only NES observer.
pub struct NesDebugger<'a> {
    core: &'a Core,
    /// `registers()` backing buffer, filled on first use. The observer
    /// is a snapshot, and must not go stale mid-frame, so callers
    /// rebuild it per use instead of holding it across frames.
    /// Laziness is pay-for-what-you-use: screen/memory-only asserts
    /// never pay for register collection.
    regs: OnceCell<Vec<(&'static str, u64)>>,
}

impl<'a> NesDebugger<'a> {
    pub fn new(core: &'a Core) -> Self {
        Self {
            core,
            regs: OnceCell::new(),
        }
    }

    fn read_byte(&self, addr: u32) -> Option<u64> {
        self.core.peek_work_ram(addr as usize).map(u64::from)
    }

    fn read_vram_byte(&self, addr: u32) -> Option<u64> {
        self.core.peek_ppu_vram(addr as usize).map(u64::from)
    }

    /// Cartridge byte with bus state: fully-driven reads as a value,
    /// floating reads as open (None). `None` also covers absent RAM,
    /// so callers distinguish open from unmapped through table
    /// coverage, never through positions.
    fn read_cartridge_byte(&self, addr: u32) -> Option<u64> {
        self.core
            .peek_cartridge_ram(addr as usize)
            .and_then(|read| (read.mask == 0xFF).then(|| u64::from(read.data)))
    }

    /// PRG byte through the same mask rule (floating reads are open).
    fn read_prg_byte(&self, addr: u32) -> Option<u64> {
        self.core.peek_prg_byte(addr as usize).map(u64::from)
    }
}

impl Debugger for NesDebugger<'_> {
    fn spaces(&self) -> &[SpaceInfo] {
        &NES_SPACES
    }

    fn space_containing(&self, addr: u32) -> Option<SpaceId> {
        NES_SPACES
            .iter()
            .find(|s| s.range.contains(&addr))
            .map(|s| s.id)
    }

    fn read(&self, space: SpaceId, addr: u32, width: u8) -> Option<u64> {
        // Single gate: unknown SpaceId, out-of-range, range-crossing,
        // and bad widths (including 0) all become None here.
        if !NES_SPACE_TABLE.covers(space, addr, width) {
            return None;
        }
        // The table holds WorkRam, PpuVram, CartridgeRam, and PrgRom;
        // covers() already pinned the id, so dispatch on (space,
        // width). A new space needs a new arm here; unknown ids cannot
        // reach this point.
        match (space, width) {
            (SPACE_WORK_RAM, 1) => self.read_byte(addr),
            (SPACE_WORK_RAM, 2) => {
                let lo = self.read_byte(addr)?;
                let hi = self.read_byte(addr + 1)?;
                Some(lo | (hi << 8))
            }
            (SPACE_WORK_RAM, 4) => {
                let mut v = 0u64;
                for i in 0..4 {
                    v |= self.read_byte(addr + i)? << (8 * i);
                }
                Some(v)
            }
            (SPACE_PPU_VRAM, 1) => self.read_vram_byte(addr),
            (SPACE_PPU_VRAM, 2) => {
                let lo = self.read_vram_byte(addr)?;
                let hi = self.read_vram_byte(addr + 1)?;
                Some(lo | (hi << 8))
            }
            (SPACE_PPU_VRAM, 4) => {
                let mut v = 0u64;
                for i in 0..4 {
                    v |= self.read_vram_byte(addr + i)? << (8 * i);
                }
                Some(v)
            }
            (SPACE_CARTRIDGE_RAM, 1) => self.read_cartridge_byte(addr),
            (SPACE_CARTRIDGE_RAM, 2) => {
                let lo = self.read_cartridge_byte(addr)?;
                let hi = self.read_cartridge_byte(addr + 1)?;
                Some(lo | (hi << 8))
            }
            (SPACE_CARTRIDGE_RAM, 4) => {
                let mut v = 0u64;
                for i in 0..4 {
                    v |= self.read_cartridge_byte(addr + i)? << (8 * i);
                }
                Some(v)
            }
            (SPACE_PRG_ROM, 1) => self.read_prg_byte(addr),
            (SPACE_PRG_ROM, 2) => {
                let lo = self.read_prg_byte(addr)?;
                let hi = self.read_prg_byte(addr + 1)?;
                Some(lo | (hi << 8))
            }
            (SPACE_PRG_ROM, 4) => {
                let mut v = 0u64;
                for i in 0..4 {
                    v |= self.read_prg_byte(addr + i)? << (8 * i);
                }
                Some(v)
            }
            _ => None,
        }
    }

    fn registers(&self) -> &[(&'static str, u64)] {
        self.regs
            .get_or_init(|| Vec::from(self.core.cpu_registers()))
    }

    fn program_counter(&self) -> Option<u32> {
        self.registers()
            .iter()
            .find(|(name, _)| *name == "pc")
            .map(|(_, value)| *value as u32)
    }

    fn panels(&self) -> Vec<DebugPanel> {
        // One panel per value kind (Bool/Hex/Dec): ColumnKind is
        // column-wide and CellValue has no empty variant, so a single
        // mixed panel cannot express this content.
        let regs = self.core.peek_ppu_debug_regs();
        let flag = |key: &'static str, set: bool| PanelRow {
            key,
            cells: vec![CellValue::Bool(set)],
        };
        let flags = DebugPanel {
            id: "ppu-flags",
            label_id: "PPU flags",
            columns: &[PanelColumn {
                id: "flag",
                label_id: "Flag",
                kind: ColumnKind::Bool,
            }],
            rows: vec![
                flag("nmi_output", regs.control & 0x80 != 0),
                flag("sprite_size_16", regs.control & 0x20 != 0),
                flag("background_table_high", regs.control & 0x10 != 0),
                flag("sprite_table_high", regs.control & 0x08 != 0),
                flag("show_background", regs.mask & 0x08 != 0),
                flag("show_sprites", regs.mask & 0x10 != 0),
                flag("grayscale", regs.mask & 0x01 != 0),
                flag("sprite_zero_hit", regs.sprite_zero_hit),
                flag("sprite_overflow", regs.sprite_overflow),
                flag("nmi_occurred", regs.nmi_occurred),
            ],
        };
        let hex = DebugPanel {
            id: "ppu-regs",
            label_id: "PPU registers",
            columns: &[PanelColumn {
                id: "value",
                label_id: "Value",
                kind: ColumnKind::Hex { digits: 2 },
            }],
            rows: vec![
                PanelRow {
                    key: "PPUCTRL",
                    cells: vec![CellValue::U64(u64::from(regs.control))],
                },
                PanelRow {
                    key: "PPUMASK",
                    cells: vec![CellValue::U64(u64::from(regs.mask))],
                },
                PanelRow {
                    key: "OAMADDR",
                    cells: vec![CellValue::U64(u64::from(regs.oam_address))],
                },
            ],
        };
        let counters = DebugPanel {
            id: "ppu-pos",
            label_id: "PPU position",
            columns: &[PanelColumn {
                id: "value",
                label_id: "Value",
                kind: ColumnKind::Dec,
            }],
            rows: vec![
                PanelRow {
                    key: "scanline",
                    cells: vec![CellValue::U64(u64::from(regs.scanline))],
                },
                PanelRow {
                    key: "cycle",
                    cells: vec![CellValue::U64(u64::from(regs.cycle))],
                },
            ],
        };
        vec![flags, hex, counters]
    }

    fn images(&self) -> Vec<DebugImage> {
        crate::pattern_tables::pattern_images(self.core)
    }

    fn disassemble(&self, addr: u32, count: u16) -> Vec<DisasmLine> {
        use crate::disasm6502::{AddrMode, decode};
        // 6502 disassembly over PRG ROM. Short reads (range end,
        // floating bus) decode as raw bytes so a truncated tail never
        // panics and never fabricates instructions.
        let pc = self.program_counter();
        let mut out = Vec::new();
        let mut cursor = addr;
        for _ in 0..count {
            if cursor > 0xFFFF {
                break;
            }
            let op = match self.read_prg_byte(cursor) {
                Some(op) => op as u8,
                None => break,
            };
            let (mnemonic, mode) = decode(op);
            let want = mode.len() as u32;
            let mut bytes = [0u8; 3];
            bytes[0] = op;
            let mut have = 1u32;
            while have < want {
                match self.read_prg_byte(cursor + have) {
                    Some(b) => {
                        bytes[have as usize] = b as u8;
                        have += 1;
                    }
                    None => break,
                }
            }
            if mode == AddrMode::Jam || have < want {
                out.push(DisasmLine {
                    addr: cursor,
                    bytes: [op, 0, 0],
                    len: 1,
                    text: format!(".DB ${op:02X}"),
                    is_pc: pc == Some(cursor),
                    // Raw bytes never navigate.
                    target: None,
                });
                cursor += 1;
                continue;
            }
            let b1 = bytes[1] as u16;
            let b2 = bytes[2] as u16;
            // Structured follow address for plain address operands
            // only: Relative, ZeroPage, and Absolute offer follow;
            // immediates, indexed and indirect forms, and implied
            // instructions do not.
            let mut target = None;
            let operand = match mode {
                AddrMode::Implied | AddrMode::Jam => String::new(),
                AddrMode::Accumulator => " A".to_string(),
                AddrMode::Immediate => format!(" #${:02X}", bytes[1]),
                AddrMode::ZeroPage => {
                    target = Some(u32::from(b1));
                    format!(" ${b1:02X}")
                }
                AddrMode::ZeroPageX => format!(" ${b1:02X},X"),
                AddrMode::ZeroPageY => format!(" ${b1:02X},Y"),
                AddrMode::Absolute => {
                    target = Some(u32::from(b1 | (b2 << 8)));
                    format!(" ${:04X}", b1 | (b2 << 8))
                }
                AddrMode::AbsoluteX => format!(" ${:04X},X", b1 | (b2 << 8)),
                AddrMode::AbsoluteY => format!(" ${:04X},Y", b1 | (b2 << 8)),
                AddrMode::Indirect => format!(" (${:04X})", b1 | (b2 << 8)),
                AddrMode::IndexedIndirect => format!(" (${b1:02X},X)"),
                AddrMode::IndirectIndexed => format!(" (${b1:02X}),Y"),
                AddrMode::Relative => {
                    let resolved = cursor.wrapping_add(2).wrapping_add(bytes[1] as i8 as u32);
                    target = Some(resolved);
                    format!(" ${resolved:04X}")
                }
            };
            out.push(DisasmLine {
                addr: cursor,
                bytes,
                len: want as u8,
                text: format!("{mnemonic}{operand}"),
                is_pc: pc == Some(cursor),
                target,
            });
            cursor += want;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use nerust_core_traits::debugger::validate_spaces;

    use super::*;
    use crate::nrom_test_data;

    fn live_core() -> Core {
        Core::new(nrom_test_data()).expect("NROM test cartridge")
    }

    #[test]
    fn nes_spaces_validate() {
        NES_SPACE_TABLE.validate().expect("NES spaces valid");
        assert_eq!(validate_spaces(&NES_SPACES), Ok(()));
    }

    #[test]
    fn nes_debugger_can_be_boxed() {
        let core = live_core();
        let boxed: Box<dyn Debugger + '_> = Box::new(NesDebugger::new(&core));
        assert_eq!(boxed.spaces().len(), 4);
        assert_eq!(boxed.space_containing(0x0100), Some(SPACE_WORK_RAM));
        assert_eq!(boxed.space_containing(0x2000), Some(SPACE_PPU_VRAM));
        assert_eq!(boxed.space_containing(0x6000), Some(SPACE_CARTRIDGE_RAM));
        assert_eq!(boxed.space_containing(0x8000), Some(SPACE_PRG_ROM));
        assert_eq!(boxed.space_containing(0x1000), Some(SPACE_WORK_RAM));
        assert_eq!(boxed.space_containing(0x4000), None);
        // Registers: 6 entries in ascending name order (kernel contract).
        let names: Vec<&str> = boxed.registers().iter().map(|(n, _)| *n).collect();
        assert_eq!(names, vec!["a", "p", "pc", "sp", "x", "y"]);
        // Program counter anchors disassembly; registers stay
        // architectural (no alias entries).
        let pc = boxed
            .registers()
            .iter()
            .find(|(name, _)| *name == "pc")
            .map(|(_, value)| *value as u32);
        assert_eq!(boxed.program_counter(), pc);
        // Live read path: fresh WRAM is readable.
        assert!(boxed.read(SPACE_WORK_RAM, 0x0100, 1).is_some());
        // Rejected inputs collapse to None.
        assert_eq!(boxed.read(SPACE_WORK_RAM, 0x1FFF, 2), None);
        assert_eq!(boxed.read(SPACE_WORK_RAM, 0x0100, 0), None);
        assert_eq!(boxed.read(SPACE_WORK_RAM, 0x0100, 3), None);
        assert_eq!(boxed.read(SpaceId(9), 0x0100, 1), None);
    }

    #[test]
    fn nes_panels_have_three_kinds() {
        let core = live_core();
        let dbg = NesDebugger::new(&core);
        let panels = dbg.panels();
        assert_eq!(panels.len(), 3);
        assert_eq!(panels[0].id, "ppu-flags");
        assert_eq!(panels[1].id, "ppu-regs");
        assert_eq!(panels[2].id, "ppu-pos");
        assert_eq!(panels[0].rows.len(), 10);
    }

    #[test]
    fn nes_pattern_images_smoke() {
        let core = live_core();
        let dbg = NesDebugger::new(&core);
        let images = dbg.images();
        assert_eq!(images.len(), 2);
        for image in &images {
            assert_eq!((image.width, image.height), (128, 128));
            assert_eq!(image.pixels.len(), 128 * 128);
            assert_eq!(image.palette.len(), 4);
        }
        // Zero CHR decodes all-blank: shape valid, content empty.
        assert!(images.iter().all(|i| i.pixels.iter().all(|&p| p == 0)));
    }

    #[test]
    fn nes_disasm_matches_nestest_trace() {
        use crate::{CartridgeData, CartridgeDataParts, MirrorMode, RomFormat};
        let rom: &[u8] = include_bytes!("../../../roms/nes-test-roms/other/nestest.nes");
        let log: &str = include_str!("../../../roms/nes-test-roms/other/nestest.log");
        assert_eq!(&rom[0..4], b"NES\x1A");
        let data = CartridgeData::new(CartridgeDataParts {
            format: RomFormat::INes,
            prog_rom: rom[16..16 + 0x4000].to_vec(),
            char_rom: rom[16 + 0x4000..16 + 0x4000 + 0x2000].to_vec(),
            pram_length: 0,
            save_pram_length: 0,
            vram_length: 0,
            save_vram_length: 0,
            mapper_type: 0,
            mirror_mode: MirrorMode::Horizontal,
            has_battery: false,
            sub_mapper_type: 0,
            trainer: Vec::new(),
        })
        .expect("nestest cartridge data should be valid");
        let core = Core::new(data).expect("nestest core");
        let debugger = NesDebugger::new(&core);
        let lines: Vec<&str> = log.lines().take(300).collect();
        assert_eq!(lines.len(), 300);
        // The trace follows control flow (jumps), so each line is
        // decoded independently at its own address.
        for line in lines {
            let addr = u32::from_str_radix(&line[0..4], 16).expect("log addr");
            // Text runs between the byte dump and the register state.
            let reg_start = line.find("  A:").expect("log registers");
            let text = line[15..reg_start].trim();
            let mut parts = text.split_whitespace();
            let mnemonic = parts.next().expect("log mnemonic");
            let operand = parts.next().unwrap_or("");
            let rows = debugger.disassemble(addr, 1);
            assert_eq!(rows.len(), 1, "no row at {addr:04X}");
            let row = &rows[0];
            assert_eq!(row.addr, addr);
            let mut got = row.text.split_whitespace();
            assert_eq!(got.next().unwrap_or(""), mnemonic, "at {line}");
            assert_eq!(got.next().unwrap_or(""), operand, "at {line}");
        }
    }

    #[test]
    fn nes_disasm_target_matches_plain_operands() {
        use crate::{CartridgeData, CartridgeDataParts, MirrorMode, RomFormat};
        let rom: &[u8] = include_bytes!("../../../roms/nes-test-roms/other/nestest.nes");
        let log: &str = include_str!("../../../roms/nes-test-roms/other/nestest.log");
        let data = CartridgeData::new(CartridgeDataParts {
            format: RomFormat::INes,
            prog_rom: rom[16..16 + 0x4000].to_vec(),
            char_rom: rom[16 + 0x4000..16 + 0x4000 + 0x2000].to_vec(),
            pram_length: 0,
            save_pram_length: 0,
            vram_length: 0,
            save_vram_length: 0,
            mapper_type: 0,
            mirror_mode: MirrorMode::Horizontal,
            has_battery: false,
            sub_mapper_type: 0,
            trainer: Vec::new(),
        })
        .expect("nestest cartridge data should be valid");
        let core = Core::new(data).expect("nestest core");
        let debugger = NesDebugger::new(&core);
        // Structured follow addresses match the trace's plain `$XXXX`
        // operands; immediates, indexed, and indirect forms stay None.
        let mut plain = 0;
        let mut other = 0;
        for line in log.lines().take(300) {
            let addr = u32::from_str_radix(&line[0..4], 16).expect("log addr");
            let reg_start = line.find("  A:").expect("log registers");
            let operand = line[15..reg_start].split_whitespace().nth(1).unwrap_or("");
            let rows = debugger.disassemble(addr, 1);
            assert_eq!(rows.len(), 1, "no row at {addr:04X}");
            let row = &rows[0];
            match operand.strip_prefix('$') {
                Some(hex) if !operand.contains(',') && !operand.contains('(') => {
                    // Plain address operand: target resolves it.
                    let expected = u32::from_str_radix(hex, 16).expect("log operand");
                    assert_eq!(row.target, Some(expected), "at {line}");
                    plain += 1;
                }
                _ => {
                    // Immediate, indexed, indirect, or implied.
                    assert_eq!(row.target, None, "at {line}");
                    other += 1;
                }
            }
        }
        // Both classes occur in the first 300 trace lines.
        assert!(plain > 0 && other > 0, "plain={plain} other={other}");
    }
}

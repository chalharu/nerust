// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod spriteinfo;
mod tileinfo;

use self::spriteinfo::SpriteInfo;
use self::tileinfo::TileInfo;
use crate::cart_device::Cartridge;
use crate::cpu::interrupt::Interrupt;
use crate::persistence::{
    DecayableOpenBusMessage, PersistenceError, PpuControlMessage, PpuMaskMessage, PpuStateMessage,
    PpuStateRegistersMessage, PpuStatusMessage, SpriteInfoMessage, TileInfoMessage,
};
use crate::ppu_memory_access::{PpuBusAccess, PpuBusEvent, PpuReadAccess};
use crate::{OpenBus, OpenBusReadResult};
use nerust_screen_traits::Screen;
use std::cmp;
use std::mem;

const NMI_SCAN_LINE: u16 = 242;
const TOTAL_SCAN_LINE: u16 = 262;
const PALETTE_ADDRESS: [usize; 32] = [
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F,
    0x00, 0x11, 0x12, 0x13, 0x04, 0x15, 0x16, 0x17, 0x08, 0x19, 0x1A, 0x1B, 0x0C, 0x1D, 0x1E, 0x1F,
];

#[derive(serde_derive::Serialize, serde_derive::Deserialize, Debug)]
struct DecayableOpenBus {
    data: u8,
    decay: [u8; 8],
}

#[cfg(test)]
mod tests {
    use super::Core;
    use crate::cart_device::Cartridge;
    use crate::cartridge;
    use crate::cpu::interrupt::Interrupt;
    use crate::{CartridgeData, CartridgeDataParts, MirrorMode, RomFormat};
    use nerust_screen_traits::Screen;

    #[derive(Default)]
    struct NullScreen;

    impl Screen for NullScreen {
        fn push(&mut self, _index: u8) {}

        fn render(&mut self) {}
    }

    fn nrom_cartridge() -> Box<dyn Cartridge> {
        let cartridge_data = CartridgeData::new(CartridgeDataParts {
            format: RomFormat::INes,
            prog_rom: vec![0; 0x8000],
            char_rom: vec![0; 0x2000],
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
        .expect("test cartridge data should be valid");
        cartridge::try_from(cartridge_data).expect("cartridge should construct")
    }

    fn mmc2_cartridge() -> Box<dyn Cartridge> {
        let character_rom = (0u8..32)
            .flat_map(|bank| std::iter::repeat_n(bank, 0x1000))
            .collect();
        let cartridge_data = CartridgeData::new(CartridgeDataParts {
            format: RomFormat::Nes20,
            prog_rom: vec![0; 0x20000],
            char_rom: character_rom,
            pram_length: 0,
            save_pram_length: 0,
            vram_length: 0,
            save_vram_length: 0,
            mapper_type: 9,
            mirror_mode: MirrorMode::Horizontal,
            has_battery: false,
            sub_mapper_type: 0,
            trainer: Vec::new(),
        })
        .expect("test cartridge data should be valid");
        cartridge::try_from(cartridge_data).expect("cartridge should construct")
    }

    #[test]
    fn background_color_zero_uses_universal_backdrop() {
        assert_eq!(Core::background_palette_index(0x00, 0), 0x00);
        assert_eq!(Core::background_palette_index(0x04, 0), 0x00);
        assert_eq!(Core::background_palette_index(0x08, 0), 0x00);
        assert_eq!(Core::background_palette_index(0x0C, 0), 0x00);
    }

    #[test]
    fn background_color_nonzero_keeps_palette_offset() {
        assert_eq!(Core::background_palette_index(0x04, 1), 0x05);
        assert_eq!(Core::background_palette_index(0x08, 2), 0x0A);
        assert_eq!(Core::background_palette_index(0x0C, 3), 0x0F);
    }

    #[test]
    fn odd_frame_skip_does_not_consume_extra_ppu_tick() {
        let mut ppu = Core::new();
        let mut cartridge = nrom_cartridge();
        let mut interrupt = Interrupt::new();
        let mut screen = NullScreen;

        ppu.scan_line = 0;
        ppu.cycle = 338;
        ppu.frames = 1;
        ppu.bus_tick = 10;
        ppu.render_executing = true;
        assert_eq!(ppu.scan_line, 0);
        assert_eq!(ppu.cycle, 338);
        assert_eq!(ppu.ppu_bus_tick(), 10);

        ppu.step(&mut screen, cartridge.as_mut(), &mut interrupt);

        assert_eq!(ppu.cycle, 340);
        assert_eq!(ppu.ppu_bus_tick(), 11);

        ppu.step(&mut screen, cartridge.as_mut(), &mut interrupt);

        assert_eq!(ppu.scan_line, 1);
        assert_eq!(ppu.cycle, 0);
        assert_eq!(ppu.ppu_bus_tick(), 11);
    }

    #[test]
    fn cpu_ppudata_reads_can_toggle_mmc2_latches() {
        let mut ppu = Core::new();
        let mut cartridge = mmc2_cartridge();
        let mut interrupt = Interrupt::new();

        cartridge.write(0xB000, 0x02, &mut interrupt);
        cartridge.write(0xC000, 0x03, &mut interrupt);
        assert_eq!(cartridge.read(0x0000).data, 0x02);

        ppu.state.vram_addr = 0x0FE8;
        let _ = ppu.read_data(cartridge.as_mut(), &mut interrupt);

        assert_eq!(cartridge.read(0x0000).data, 0x03);
    }
}

impl DecayableOpenBus {
    pub(crate) fn new() -> Self {
        Self {
            data: 0,
            decay: [0; 8],
        }
    }

    pub(crate) fn unite(&mut self, data: OpenBusReadResult) -> u8 {
        for i in 0..8 {
            if (data.mask >> i) == 1 {
                self.decay[i] = 20;
            }
        }
        let result = (self.data & !data.mask) | (data.data & data.mask);
        self.data = result;
        result
    }

    pub(crate) fn write(&mut self, data: u8) -> u8 {
        self.data = data;
        self.decay = [20; 8];
        data
    }

    pub(crate) fn next(&mut self) {
        let mut result_mask: u8 = 0;
        for i in 0..8 {
            if self.decay[i] > 0 {
                self.decay[i] -= 1;
                result_mask |= 1 << i;
            }
        }
        self.data &= result_mask;
    }

    fn export_state_proto(&self) -> DecayableOpenBusMessage {
        DecayableOpenBusMessage {
            data: u32::from(self.data),
            decay: self.decay.iter().copied().map(u32::from).collect(),
        }
    }

    fn import_state_proto(
        &mut self,
        payload: &DecayableOpenBusMessage,
    ) -> Result<(), PersistenceError> {
        if payload.decay.len() != self.decay.len() {
            return Err(PersistenceError::Validation(
                "PPU decayable open bus length mismatch".into(),
            ));
        }
        self.data = u8::try_from(payload.data)
            .map_err(|_| PersistenceError::Validation("PPU IO open bus overflow".into()))?;
        for (slot, value) in self.decay.iter_mut().zip(payload.decay.iter().copied()) {
            *slot = u8::try_from(value)
                .map_err(|_| PersistenceError::Validation("PPU decay counter overflow".into()))?;
        }
        Ok(())
    }
}

#[derive(serde_derive::Serialize, serde_derive::Deserialize, Debug)]
struct State {
    control: u8,
    mask: u8,
    oam_address: u8,

    vram_addr: u16,
    temp_vram_addr: u16,
    x_scroll: u8,
    write_toggle: bool,

    high_bit_shift: u16,
    low_bit_shift: u16,
}

impl State {
    pub(crate) fn new() -> Self {
        Self {
            control: 0,
            mask: 0,
            oam_address: 0,
            vram_addr: 0,
            temp_vram_addr: 0,
            x_scroll: 0,
            write_toggle: false,
            high_bit_shift: 0,
            low_bit_shift: 0,
        }
    }

    pub(crate) fn reset(&mut self) {
        self.control = 0;
        self.mask = 0;
        self.oam_address = 0;
        self.vram_addr = 0;
        self.temp_vram_addr = 0;
        self.x_scroll = 0;
        self.write_toggle = false;
        self.high_bit_shift = 0;
        self.low_bit_shift = 0;
    }

    fn name_table_address(&self) -> usize {
        0x2000 | usize::from(self.vram_addr & 0x0FFF)
    }

    fn attribute_address(&self) -> usize {
        usize::from(
            0x23C0
                | (self.vram_addr & 0x0C00)
                | ((self.vram_addr >> 4) & 0x38)
                | ((self.vram_addr >> 2) & 0x07),
        )
    }
}

#[derive(serde_derive::Serialize, serde_derive::Deserialize, Debug)]
struct Control {
    name_table: u8,
    increment: bool,
    sprite_table: bool,
    background_table: bool,
    sprite_size: bool,
    master_slave: bool,
    nmi_output: bool,
}

impl Control {
    pub(crate) fn new() -> Self {
        Self {
            name_table: 0,
            increment: false,
            sprite_table: false,
            background_table: false,
            sprite_size: false,
            master_slave: false,
            nmi_output: false,
        }
    }

    pub(crate) fn reset(&mut self) {
        self.name_table = 0;
        self.increment = false;
        self.sprite_table = false;
        self.background_table = false;
        self.sprite_size = false;
        self.master_slave = false;
        self.nmi_output = false;
    }
}

impl From<u8> for Control {
    fn from(value: u8) -> Self {
        Self {
            name_table: value & 3,
            increment: value & 4 != 0,
            sprite_table: value & 8 != 0,
            background_table: value & 0x10 != 0,
            sprite_size: value & 0x20 != 0,
            master_slave: value & 0x40 != 0,
            nmi_output: value & 0x80 != 0,
        }
    }
}

#[derive(serde_derive::Serialize, serde_derive::Deserialize, Debug)]
struct Mask {
    grayscale: bool,
    show_left_background: bool,
    show_left_sprites: bool,
    show_background: bool,
    show_sprites: bool,
    red_tint: bool,
    green_tint: bool,
    blue_tint: bool,
}

impl Mask {
    pub(crate) fn new() -> Self {
        Self {
            grayscale: false,
            show_left_background: false,
            show_left_sprites: false,
            show_background: false,
            show_sprites: false,
            red_tint: false,
            green_tint: false,
            blue_tint: false,
        }
    }

    pub(crate) fn reset(&mut self) {
        self.grayscale = false;
        self.show_left_background = false;
        self.show_left_sprites = false;
        self.show_background = false;
        self.show_sprites = false;
        self.red_tint = false;
        self.green_tint = false;
        self.blue_tint = false;
    }
}

impl From<u8> for Mask {
    fn from(value: u8) -> Self {
        Self {
            grayscale: value & 1 != 0,
            show_left_background: value & 2 != 0,
            show_left_sprites: value & 4 != 0,
            show_background: value & 8 != 0,
            show_sprites: value & 0x10 != 0,
            red_tint: value & 0x20 != 0,
            green_tint: value & 0x40 != 0,
            blue_tint: value & 0x80 != 0,
        }
    }
}

#[derive(serde_derive::Serialize, serde_derive::Deserialize, Debug)]
struct Status {
    sprite_zero_hit: bool,
    sprite_overflow: bool,
    nmi_occurred: bool,
}

impl Status {
    pub(crate) fn new() -> Self {
        Self {
            sprite_zero_hit: false,
            sprite_overflow: false,
            nmi_occurred: false,
        }
    }

    pub(crate) fn reset(&mut self) {
        self.sprite_zero_hit = false;
        self.sprite_overflow = false;
        self.nmi_occurred = false;
    }
}

#[derive(serde_derive::Serialize, serde_derive::Deserialize)]
pub(crate) struct Core {
    // memory
    #[serde(with = "nerust_serialize::BigArray")]
    vram: [u8; 2048],
    palette: [u8; 32],

    state: State,
    cycle: u16,
    scan_line: u16,
    frames: usize,
    #[serde(default)]
    bus_tick: u64,
    buffered_data: u8,

    #[serde(with = "nerust_serialize::BigArray")]
    primary_oam: [u8; 256],
    secondary_oam: [u8; 32],
    secondary_oam_address: u8,

    control: Control,
    mask: Mask,
    status: Status,

    current_tile: TileInfo,
    previous_tile: TileInfo,
    next_tile: TileInfo,
    #[serde(with = "nerust_serialize::BigArray")]
    sprites: [SpriteInfo; 64],
    sprite_index: u8,
    sprite_count: u8,

    render_executing: bool,
    post_render_executing: bool,

    oam_read_buffer: u8,
    vram_read_delay: u8,

    vram_addr_update_delay: u8,
    new_vram_addr: u16,

    has_first_sprite_next: bool,
    has_first_sprite: bool,
    has_sprite: bool,
    sprite_overflow_delay: u8,
    sprite_reading: bool,
    oam_address_high: u8,
    oam_address_low: u8,
    openbus_vram: OpenBus,
    openbus_io: DecayableOpenBus,
    has_next_sprite: bool,
    // screen_buffer: [u8; 256 * 240],
}

impl Core {
    pub(crate) fn new() -> Self {
        Self {
            vram: [0; 2048],
            palette: [
                0x09, 0x01, 0x00, 0x01, 0x00, 0x02, 0x02, 0x0D, 0x08, 0x10, 0x08, 0x24, 0x00, 0x00,
                0x04, 0x2C, 0x09, 0x01, 0x34, 0x03, 0x00, 0x04, 0x00, 0x14, 0x08, 0x3A, 0x00, 0x02,
                0x00, 0x20, 0x2C, 0x08,
            ],
            state: State::new(),
            cycle: 0,
            scan_line: 0,
            frames: 0,
            bus_tick: 0,
            buffered_data: 0,
            primary_oam: [0; 256],
            secondary_oam: [0; 32],
            control: Control::new(),
            mask: Mask::new(),
            status: Status::new(),
            current_tile: TileInfo::new(),
            previous_tile: TileInfo::new(),
            next_tile: TileInfo::new(),
            sprites: [SpriteInfo::new(); 64],
            render_executing: false,
            post_render_executing: false,
            oam_read_buffer: 0,
            vram_read_delay: 0,
            vram_addr_update_delay: 0,
            new_vram_addr: 0,
            has_first_sprite: false,
            has_first_sprite_next: false,
            has_sprite: false,
            sprite_overflow_delay: 0,
            oam_address_high: 0,
            oam_address_low: 0,
            sprite_reading: false,
            secondary_oam_address: 0,
            sprite_count: 0,
            sprite_index: 0,
            openbus_vram: OpenBus::new(),
            openbus_io: DecayableOpenBus::new(),
            has_next_sprite: false,
            // screen_buffer: [0; 256 * 240],
        }
    }

    pub(crate) fn export_state_proto(&self) -> PpuStateMessage {
        PpuStateMessage {
            vram: self.vram.to_vec(),
            palette: self.palette.to_vec(),
            state: Some(PpuStateRegistersMessage {
                control: u32::from(self.state.control),
                mask: u32::from(self.state.mask),
                oam_address: u32::from(self.state.oam_address),
                vram_addr: u32::from(self.state.vram_addr),
                temp_vram_addr: u32::from(self.state.temp_vram_addr),
                x_scroll: u32::from(self.state.x_scroll),
                write_toggle: self.state.write_toggle,
                high_bit_shift: u32::from(self.state.high_bit_shift),
                low_bit_shift: u32::from(self.state.low_bit_shift),
            }),
            cycle: u32::from(self.cycle),
            scan_line: u32::from(self.scan_line),
            frames: self.frames as u64,
            bus_tick: self.bus_tick,
            buffered_data: u32::from(self.buffered_data),
            primary_oam: self.primary_oam.to_vec(),
            secondary_oam: self.secondary_oam.to_vec(),
            secondary_oam_address: u32::from(self.secondary_oam_address),
            control: Some(PpuControlMessage {
                name_table: u32::from(self.control.name_table),
                increment: self.control.increment,
                sprite_table: self.control.sprite_table,
                background_table: self.control.background_table,
                sprite_size: self.control.sprite_size,
                master_slave: self.control.master_slave,
                nmi_output: self.control.nmi_output,
            }),
            mask: Some(PpuMaskMessage {
                grayscale: self.mask.grayscale,
                show_left_background: self.mask.show_left_background,
                show_left_sprites: self.mask.show_left_sprites,
                show_background: self.mask.show_background,
                show_sprites: self.mask.show_sprites,
                red_tint: self.mask.red_tint,
                green_tint: self.mask.green_tint,
                blue_tint: self.mask.blue_tint,
            }),
            status: Some(PpuStatusMessage {
                sprite_zero_hit: self.status.sprite_zero_hit,
                sprite_overflow: self.status.sprite_overflow,
                nmi_occurred: self.status.nmi_occurred,
            }),
            current_tile: Some(Self::tile_info_to_proto(self.current_tile)),
            previous_tile: Some(Self::tile_info_to_proto(self.previous_tile)),
            next_tile: Some(Self::tile_info_to_proto(self.next_tile)),
            sprites: self
                .sprites
                .into_iter()
                .map(Self::sprite_info_to_proto)
                .collect(),
            sprite_index: u32::from(self.sprite_index),
            sprite_count: u32::from(self.sprite_count),
            render_executing: self.render_executing,
            post_render_executing: self.post_render_executing,
            oam_read_buffer: u32::from(self.oam_read_buffer),
            vram_read_delay: u32::from(self.vram_read_delay),
            vram_addr_update_delay: u32::from(self.vram_addr_update_delay),
            new_vram_addr: u32::from(self.new_vram_addr),
            has_first_sprite_next: self.has_first_sprite_next,
            has_first_sprite: self.has_first_sprite,
            has_sprite: self.has_sprite,
            sprite_overflow_delay: u32::from(self.sprite_overflow_delay),
            sprite_reading: self.sprite_reading,
            oam_address_high: u32::from(self.oam_address_high),
            oam_address_low: u32::from(self.oam_address_low),
            openbus_vram_data: u32::from(self.openbus_vram.data),
            openbus_io: Some(self.openbus_io.export_state_proto()),
            has_next_sprite: self.has_next_sprite,
        }
    }

    pub(crate) fn import_state_proto(
        &mut self,
        payload: &PpuStateMessage,
    ) -> Result<(), PersistenceError> {
        if payload.vram.len() != self.vram.len()
            || payload.palette.len() != self.palette.len()
            || payload.primary_oam.len() != self.primary_oam.len()
            || payload.secondary_oam.len() != self.secondary_oam.len()
            || payload.sprites.len() != self.sprites.len()
        {
            return Err(PersistenceError::Validation(
                "PPU state length mismatch".into(),
            ));
        }
        self.vram.copy_from_slice(&payload.vram);
        self.palette.copy_from_slice(&payload.palette);
        let state = payload
            .state
            .as_ref()
            .ok_or_else(|| PersistenceError::Validation("missing PPU register state".into()))?;
        self.state.control = u8::try_from(state.control)
            .map_err(|_| PersistenceError::Validation("PPU control overflow".into()))?;
        self.state.mask = u8::try_from(state.mask)
            .map_err(|_| PersistenceError::Validation("PPU mask overflow".into()))?;
        self.state.oam_address = u8::try_from(state.oam_address)
            .map_err(|_| PersistenceError::Validation("PPU OAM address overflow".into()))?;
        self.state.vram_addr = u16::try_from(state.vram_addr)
            .map_err(|_| PersistenceError::Validation("PPU VRAM address overflow".into()))?;
        self.state.temp_vram_addr = u16::try_from(state.temp_vram_addr)
            .map_err(|_| PersistenceError::Validation("PPU temp VRAM address overflow".into()))?;
        self.state.x_scroll = u8::try_from(state.x_scroll)
            .map_err(|_| PersistenceError::Validation("PPU x scroll overflow".into()))?;
        self.state.write_toggle = state.write_toggle;
        self.state.high_bit_shift = u16::try_from(state.high_bit_shift)
            .map_err(|_| PersistenceError::Validation("PPU high shift overflow".into()))?;
        self.state.low_bit_shift = u16::try_from(state.low_bit_shift)
            .map_err(|_| PersistenceError::Validation("PPU low shift overflow".into()))?;
        self.cycle = u16::try_from(payload.cycle)
            .map_err(|_| PersistenceError::Validation("PPU cycle overflow".into()))?;
        self.scan_line = u16::try_from(payload.scan_line)
            .map_err(|_| PersistenceError::Validation("PPU scanline overflow".into()))?;
        self.frames = usize::try_from(payload.frames)
            .map_err(|_| PersistenceError::Validation("PPU frame counter overflow".into()))?;
        self.bus_tick = payload.bus_tick;
        self.buffered_data = u8::try_from(payload.buffered_data)
            .map_err(|_| PersistenceError::Validation("PPU buffered data overflow".into()))?;
        self.primary_oam.copy_from_slice(&payload.primary_oam);
        self.secondary_oam.copy_from_slice(&payload.secondary_oam);
        self.secondary_oam_address = u8::try_from(payload.secondary_oam_address).map_err(|_| {
            PersistenceError::Validation("PPU secondary OAM address overflow".into())
        })?;
        let control = payload
            .control
            .as_ref()
            .ok_or_else(|| PersistenceError::Validation("missing PPU control state".into()))?;
        self.control.name_table = u8::try_from(control.name_table)
            .map_err(|_| PersistenceError::Validation("PPU control name table overflow".into()))?;
        self.control.increment = control.increment;
        self.control.sprite_table = control.sprite_table;
        self.control.background_table = control.background_table;
        self.control.sprite_size = control.sprite_size;
        self.control.master_slave = control.master_slave;
        self.control.nmi_output = control.nmi_output;
        let mask = payload
            .mask
            .as_ref()
            .ok_or_else(|| PersistenceError::Validation("missing PPU mask state".into()))?;
        self.mask.grayscale = mask.grayscale;
        self.mask.show_left_background = mask.show_left_background;
        self.mask.show_left_sprites = mask.show_left_sprites;
        self.mask.show_background = mask.show_background;
        self.mask.show_sprites = mask.show_sprites;
        self.mask.red_tint = mask.red_tint;
        self.mask.green_tint = mask.green_tint;
        self.mask.blue_tint = mask.blue_tint;
        let status = payload
            .status
            .as_ref()
            .ok_or_else(|| PersistenceError::Validation("missing PPU status state".into()))?;
        self.status.sprite_zero_hit = status.sprite_zero_hit;
        self.status.sprite_overflow = status.sprite_overflow;
        self.status.nmi_occurred = status.nmi_occurred;
        self.current_tile = Self::tile_info_from_proto(
            payload
                .current_tile
                .as_ref()
                .ok_or_else(|| PersistenceError::Validation("missing current PPU tile".into()))?,
        )?;
        self.previous_tile =
            Self::tile_info_from_proto(payload.previous_tile.as_ref().ok_or_else(|| {
                PersistenceError::Validation("missing previous PPU tile".into())
            })?)?;
        self.next_tile = Self::tile_info_from_proto(
            payload
                .next_tile
                .as_ref()
                .ok_or_else(|| PersistenceError::Validation("missing next PPU tile".into()))?,
        )?;
        for (slot, sprite) in self.sprites.iter_mut().zip(payload.sprites.iter()) {
            *slot = Self::sprite_info_from_proto(sprite)?;
        }
        self.sprite_index = u8::try_from(payload.sprite_index)
            .map_err(|_| PersistenceError::Validation("PPU sprite index overflow".into()))?;
        self.sprite_count = u8::try_from(payload.sprite_count)
            .map_err(|_| PersistenceError::Validation("PPU sprite count overflow".into()))?;
        self.render_executing = payload.render_executing;
        self.post_render_executing = payload.post_render_executing;
        self.oam_read_buffer = u8::try_from(payload.oam_read_buffer)
            .map_err(|_| PersistenceError::Validation("PPU OAM read buffer overflow".into()))?;
        self.vram_read_delay = u8::try_from(payload.vram_read_delay)
            .map_err(|_| PersistenceError::Validation("PPU VRAM delay overflow".into()))?;
        self.vram_addr_update_delay = u8::try_from(payload.vram_addr_update_delay)
            .map_err(|_| PersistenceError::Validation("PPU VRAM update delay overflow".into()))?;
        self.new_vram_addr = u16::try_from(payload.new_vram_addr)
            .map_err(|_| PersistenceError::Validation("PPU new VRAM address overflow".into()))?;
        self.has_first_sprite_next = payload.has_first_sprite_next;
        self.has_first_sprite = payload.has_first_sprite;
        self.has_sprite = payload.has_sprite;
        self.sprite_overflow_delay = u8::try_from(payload.sprite_overflow_delay).map_err(|_| {
            PersistenceError::Validation("PPU sprite overflow delay overflow".into())
        })?;
        self.sprite_reading = payload.sprite_reading;
        self.oam_address_high = u8::try_from(payload.oam_address_high)
            .map_err(|_| PersistenceError::Validation("PPU OAM address high overflow".into()))?;
        self.oam_address_low = u8::try_from(payload.oam_address_low)
            .map_err(|_| PersistenceError::Validation("PPU OAM address low overflow".into()))?;
        self.openbus_vram = OpenBus {
            data: u8::try_from(payload.openbus_vram_data)
                .map_err(|_| PersistenceError::Validation("PPU VRAM open bus overflow".into()))?,
        };
        self.openbus_io.import_state_proto(
            payload
                .openbus_io
                .as_ref()
                .ok_or_else(|| PersistenceError::Validation("missing PPU IO open bus".into()))?,
        )?;
        self.has_next_sprite = payload.has_next_sprite;
        Ok(())
    }

    pub(crate) fn reset(&mut self) {
        self.vram = [0; 2048];
        self.palette = [
            0x09, 0x01, 0x00, 0x01, 0x00, 0x02, 0x02, 0x0D, 0x08, 0x10, 0x08, 0x24, 0x00, 0x00,
            0x04, 0x2C, 0x09, 0x01, 0x34, 0x03, 0x00, 0x04, 0x00, 0x14, 0x08, 0x3A, 0x00, 0x02,
            0x00, 0x20, 0x2C, 0x08,
        ];
        self.state.reset();
        self.cycle = 0;
        self.scan_line = 0;
        self.frames = 0;
        self.bus_tick = 0;
        self.buffered_data = 0;
        self.primary_oam = [0; 256];
        self.secondary_oam = [0; 32];
        self.control.reset();
        self.mask.reset();
        self.status.reset();
        self.current_tile.reset();
        self.previous_tile.reset();
        self.next_tile.reset();
        self.sprites = [SpriteInfo::new(); 64];
        self.has_first_sprite = false;
        // self.render_executing = false;
        // self.post_render_executing = false;
        // self.oam_read_buffer = 0;
        self.has_next_sprite = false;
    }

    fn tile_info_to_proto(value: TileInfo) -> TileInfoMessage {
        TileInfoMessage {
            low_byte: u32::from(value.low_byte),
            high_byte: u32::from(value.high_byte),
            palette_offset: u32::from(value.palette_offset),
            tile_addr: u32::from(value.tile_addr),
        }
    }

    fn tile_info_from_proto(payload: &TileInfoMessage) -> Result<TileInfo, PersistenceError> {
        Ok(TileInfo {
            low_byte: u8::try_from(payload.low_byte)
                .map_err(|_| PersistenceError::Validation("PPU tile low byte overflow".into()))?,
            high_byte: u8::try_from(payload.high_byte)
                .map_err(|_| PersistenceError::Validation("PPU tile high byte overflow".into()))?,
            palette_offset: u8::try_from(payload.palette_offset).map_err(|_| {
                PersistenceError::Validation("PPU tile palette offset overflow".into())
            })?,
            tile_addr: u16::try_from(payload.tile_addr)
                .map_err(|_| PersistenceError::Validation("PPU tile address overflow".into()))?,
        })
    }

    fn sprite_info_to_proto(value: SpriteInfo) -> SpriteInfoMessage {
        SpriteInfoMessage {
            low_byte: u32::from(value.low_byte),
            high_byte: u32::from(value.high_byte),
            palette_offset: u32::from(value.palette_offset),
            tile_addr: u32::from(value.tile_addr),
            horizontal_mirror: value.horizontal_mirror,
            priority: value.priority,
            position: u32::from(value.position),
        }
    }

    fn sprite_info_from_proto(payload: &SpriteInfoMessage) -> Result<SpriteInfo, PersistenceError> {
        Ok(SpriteInfo {
            low_byte: u8::try_from(payload.low_byte)
                .map_err(|_| PersistenceError::Validation("PPU sprite low byte overflow".into()))?,
            high_byte: u8::try_from(payload.high_byte).map_err(|_| {
                PersistenceError::Validation("PPU sprite high byte overflow".into())
            })?,
            palette_offset: u8::try_from(payload.palette_offset).map_err(|_| {
                PersistenceError::Validation("PPU sprite palette offset overflow".into())
            })?,
            tile_addr: u16::try_from(payload.tile_addr).map_err(|_| {
                PersistenceError::Validation("PPU sprite tile address overflow".into())
            })?,
            horizontal_mirror: payload.horizontal_mirror,
            priority: payload.priority,
            position: u8::try_from(payload.position)
                .map_err(|_| PersistenceError::Validation("PPU sprite position overflow".into()))?,
        })
    }

    pub(crate) fn peek_vram(&self, mut address: usize, cartridge: &dyn Cartridge) -> Option<u8> {
        address &= 0x3FFF;
        match address {
            0x2000..=0x3EFF => cartridge.peek_ppu_nametable(address, &self.vram),
            0x3F00..=0x3FFF => Some(self.palette[Self::palette_address(address)]),
            _ => None,
        }
    }

    #[inline]
    pub(crate) fn read_register(
        &mut self,
        address: usize,
        cartridge: &mut dyn Cartridge,
        interrupt: &mut Interrupt,
    ) -> OpenBusReadResult {
        let decoded = 0x2000 + (address & 7);
        let result = match decoded {
            0x2002 => {
                let status = self.read_status(interrupt);
                if address == 0x2002 {
                    cartridge.notify_ppu_status_read(status, interrupt);
                }
                OpenBusReadResult::new(status, 0b1110_0000)
            }
            0x2004 => OpenBusReadResult::new(self.read_oam(), 0xFF),
            0x2007 => self.read_data(cartridge, interrupt),
            _ => OpenBusReadResult::new(0, 0),
        };
        OpenBusReadResult::new(self.openbus_io.unite(result), 0xFF)
    }

    #[inline]
    fn read_status(&mut self, interrupt: &mut Interrupt) -> u8 {
        self.state.write_toggle = false;
        self.mask.blue_tint = false;

        let result = if self.status.sprite_overflow { 0x20 } else { 0 }
            | if self.status.sprite_zero_hit { 0x40 } else { 0 }
            | if self.status.nmi_occurred && (self.scan_line != 242 || self.cycle != 0) {
                0x80
            } else {
                0
            };
        self.status.nmi_occurred = false;

        if self.scan_line == 242 && self.cycle < 3 {
            interrupt.nmi = false;
        }
        result
    }

    #[inline]
    fn read_oam(&mut self) -> u8 {
        if self.scan_line <= 240 && self.render_executing {
            if self.cycle >= 257 && self.cycle <= 320 {
                self.secondary_oam_address = (((self.cycle - 257) >> 1) as u8 & 0xFC)
                    + cmp::min((self.cycle - 257) as u8 & 7, 3);
                self.oam_read_buffer = self.secondary_oam[usize::from(self.secondary_oam_address)];
            }
            self.oam_read_buffer
        } else {
            self.primary_oam[usize::from(self.state.oam_address)]
        }
    }

    #[inline]
    fn read_data(
        &mut self,
        cartridge: &mut dyn Cartridge,
        interrupt: &mut Interrupt,
    ) -> OpenBusReadResult {
        if self.vram_read_delay > 0 {
            OpenBusReadResult::new(0, 0)
        } else {
            self.vram_read_delay = 6;
            let addr = self.state.vram_addr as usize;
            let mut value =
                self.read_vram_internal(addr, cartridge, interrupt, false, PpuReadAccess::CpuData);
            // emulate buffered reads
            let mask = if (addr & 0x3FFF) < 0x3F00 {
                mem::swap(&mut self.buffered_data, &mut value);
                0xFF
            } else {
                // let buffered_data = self.buffered_data;
                self.buffered_data = value;
                // value = self.read_palette(addr) | (buffered_data & 0xC0);
                value = self.read_palette(addr);
                0x3F
            };
            self.increment_address(cartridge, interrupt);
            OpenBusReadResult::new(value, mask)
        }
    }

    #[inline]
    fn increment_address(&mut self, cartridge: &mut dyn Cartridge, interrupt: &mut Interrupt) {
        if self.scan_line > 240 || !self.render_executing {
            self.state.vram_addr =
                (self.state.vram_addr + if self.control.increment { 32 } else { 1 }) & 0x7FFF;
            let _ = self.read_vram_internal(
                self.state.vram_addr as usize,
                cartridge,
                interrupt,
                true,
                PpuReadAccess::CpuData,
            );
        } else {
            self.increment_x();
            self.increment_y();
        }
    }

    #[inline]
    fn increment_x(&mut self) {
        if self.state.vram_addr & 0x1F == 0x1F {
            self.state.vram_addr = (self.state.vram_addr & 0xFFE0) ^ 0x0400;
        } else {
            self.state.vram_addr = self.state.vram_addr.wrapping_add(1);
        }
    }

    #[inline]
    fn increment_y(&mut self) {
        if self.state.vram_addr & 0x7000 != 0x7000 {
            self.state.vram_addr = self.state.vram_addr.wrapping_add(0x1000);
        } else {
            self.state.vram_addr &= 0x8FFF;
            let x = match (self.state.vram_addr & 0x03E0) >> 5 {
                29 => {
                    self.state.vram_addr ^= 0x800;
                    0
                }
                31 => 0,
                y => y + 1,
            } << 5;
            self.state.vram_addr = (self.state.vram_addr & 0xFC1F) | x;
        }
    }

    #[inline]
    fn ppu_bus_tick(&self) -> u64 {
        self.bus_tick
    }

    #[inline]
    pub(crate) fn read_vram(
        &mut self,
        address: usize,
        cartridge: &mut dyn Cartridge,
        interrupt: &mut Interrupt,
        access: PpuReadAccess,
    ) -> u8 {
        self.read_vram_internal(address, cartridge, interrupt, false, access)
    }

    #[inline]
    fn read_vram_internal(
        &mut self,
        mut address: usize,
        cartridge: &mut dyn Cartridge,
        interrupt: &mut Interrupt,
        address_register_change: bool,
        access: PpuReadAccess,
    ) -> u8 {
        address &= 0x3FFF;
        if self.render_executing || address <= 0x1FFF {
            cartridge.notify_ppu_bus_event(
                PpuBusEvent::AddressBusUpdate {
                    address,
                    ppu_tick: self.ppu_bus_tick(),
                    from_cpu_register: address_register_change,
                    access: PpuBusAccess::Read,
                },
                interrupt,
            );
        }
        let result = match address {
            0..=0x1FFF => cartridge.read_ppu_pattern(address, access, interrupt),
            0x2000..=0x3EFF => cartridge.read_ppu_nametable(address, access, &mut self.vram),
            0x3F00..=0x3FFF => {
                let mirrored_nametable = 0x2000 | ((address - 0x1000) & 0x0FFF);
                cartridge.read_ppu_nametable(
                    mirrored_nametable,
                    PpuReadAccess::CpuData,
                    &mut self.vram,
                )
            }
            _ => {
                log::error!("unhandled ppu memory read at address: 0x{:04X}", address);
                OpenBusReadResult::new(0, 0)
            }
        };
        self.openbus_vram.unite(result)
    }

    #[inline]
    fn palette_address(address: usize) -> usize {
        PALETTE_ADDRESS[address & 0x1F]
    }

    #[inline]
    fn read_palette(&self, address: usize) -> u8 {
        self.palette[Self::palette_address(address)]
    }

    #[inline]
    fn write_palette(&mut self, address: usize, value: u8) {
        self.palette[Self::palette_address(address)] = value & 0x3F;
    }

    #[inline]
    pub(crate) fn write_register(
        &mut self,
        address: usize,
        value: u8,
        cartridge: &mut dyn Cartridge,
        interrupt: &mut Interrupt,
    ) {
        match 0x2000 + (address & 7) {
            0x2000 => self.write_control(address, value, cartridge, interrupt),
            0x2001 => self.write_mask(address, value, cartridge),
            0x2003 => self.write_oam_address(value),
            0x2004 => self.write_oam_data(value),
            0x2005 => self.write_scroll(value),
            0x2006 => self.write_address(value),
            0x2007 => self.write_data(value, cartridge, interrupt),
            _ => {}
        }
        let _ = self.openbus_io.write(value);
    }

    #[inline]
    fn write_control(
        &mut self,
        address: usize,
        value: u8,
        cartridge: &mut dyn Cartridge,
        interrupt: &mut Interrupt,
    ) {
        let prev_nmi_output = self.control.nmi_output;

        self.control = Control::from(value);
        if address == 0x2000 {
            cartridge.notify_ppu_ctrl(value);
        }

        // 画面の書き換え場所を変更
        self.state.temp_vram_addr =
            (self.state.temp_vram_addr & 0xF3FF) | (u16::from(self.control.name_table) << 10);
        if !prev_nmi_output
            && self.control.nmi_output
            && self.status.nmi_occurred
            && (self.scan_line != 0 || self.cycle != 0)
        {
            interrupt.nmi = true;
        }
        if self.scan_line == 242 && self.cycle < 3 && !self.control.nmi_output {
            interrupt.nmi = false;
        }
    }

    #[inline]
    fn write_mask(&mut self, address: usize, value: u8, cartridge: &mut dyn Cartridge) {
        self.mask = Mask::from(value);
        if address == 0x2001 {
            cartridge.notify_ppu_mask(value);
        }
    }

    #[inline]
    fn write_oam_address(&mut self, value: u8) {
        self.state.oam_address = value;
    }

    #[inline]
    fn write_oam_data(&mut self, value: u8) {
        if self.scan_line > 240 || !self.render_executing {
            self.primary_oam[usize::from(self.state.oam_address)] =
                if (self.state.oam_address & 0x03) == 0x02 {
                    value & 0xE3
                } else {
                    value
                };
            self.state.oam_address = self.state.oam_address.wrapping_add(1);
        } else {
            self.state.oam_address = self.state.oam_address.wrapping_add(4);
        }
    }

    #[inline]
    fn write_scroll(&mut self, value: u8) {
        if self.state.write_toggle {
            self.state.temp_vram_addr = (self.state.temp_vram_addr & !0x73E0)
                | (u16::from(value & 0x07) << 12)
                | (u16::from(value & 0xF8) << 2);
        } else {
            self.state.temp_vram_addr =
                (self.state.temp_vram_addr & 0xFFE0) | u16::from(value >> 3);
            self.state.x_scroll = value & 0x07;
        }
        self.state.write_toggle = !self.state.write_toggle;
    }

    #[inline]
    fn write_address(&mut self, value: u8) {
        if self.state.write_toggle {
            self.state.temp_vram_addr = (self.state.temp_vram_addr & 0xFF00) | u16::from(value);
            self.vram_addr_update_delay = 2;
            self.new_vram_addr = self.state.temp_vram_addr;
        } else {
            self.state.temp_vram_addr =
                (self.state.temp_vram_addr & 0x80FF) | (u16::from(value & 0x3F) << 8);
        }
        self.state.write_toggle = !self.state.write_toggle;
    }

    #[inline]
    fn write_data(&mut self, value: u8, cartridge: &mut dyn Cartridge, interrupt: &mut Interrupt) {
        let addr = (self.state.vram_addr & 0x3FFF) as usize;

        if addr < 0x3F00 {
            self.write_vram_internal(addr, value, cartridge, interrupt, false);
        } else {
            self.write_palette(addr, value);
        }
        self.increment_address(cartridge, interrupt);
    }

    #[inline]
    fn write_vram(
        &mut self,
        address: usize,
        value: u8,
        cartridge: &mut dyn Cartridge,
        interrupt: &mut Interrupt,
    ) {
        self.write_vram_internal(address, value, cartridge, interrupt, false);
    }

    #[inline]
    fn write_vram_internal(
        &mut self,
        mut address: usize,
        value: u8,
        cartridge: &mut dyn Cartridge,
        interrupt: &mut Interrupt,
        address_register_change: bool,
    ) {
        address &= 0x3FFF;
        if self.render_executing || address <= 0x1FFF {
            cartridge.notify_ppu_bus_event(
                PpuBusEvent::AddressBusUpdate {
                    address,
                    ppu_tick: self.ppu_bus_tick(),
                    from_cpu_register: address_register_change,
                    access: PpuBusAccess::Write,
                },
                interrupt,
            );
        }
        match address {
            0..=0x1FFF => cartridge.write_ppu_pattern(address, value, interrupt),
            0x2000..=0x3EFF => {
                cartridge.write_ppu_nametable(address, value, &mut self.vram, interrupt)
            }
            _ => log::error!("unhandled ppu memory write at address: 0x{:04X}", address),
        }
    }

    #[inline]
    fn fetch_name_table_byte(&mut self, cartridge: &mut dyn Cartridge, interrupt: &mut Interrupt) {
        self.previous_tile = mem::replace(&mut self.current_tile, self.next_tile);
        self.state.low_bit_shift |= u16::from(self.next_tile.low_byte);
        self.state.high_bit_shift |= u16::from(self.next_tile.high_byte);
        self.next_tile.tile_addr = (u16::from(self.read_vram(
            self.state.name_table_address(),
            cartridge,
            interrupt,
            PpuReadAccess::BackgroundNameTable,
        )) << 4)
            | ((self.state.vram_addr >> 12) & 7)
            | if self.control.background_table {
                0x1000
            } else {
                0
            };
    }

    #[inline]
    fn fetch_attribute_table_byte(
        &mut self,
        cartridge: &mut dyn Cartridge,
        interrupt: &mut Interrupt,
    ) {
        let v = self.state.vram_addr as usize;
        let address = self.state.attribute_address();
        let shift = ((v >> 4) & 4) | (v & 2);
        self.next_tile.palette_offset = ((self.read_vram(
            address,
            cartridge,
            interrupt,
            PpuReadAccess::BackgroundAttribute,
        ) >> shift)
            & 3)
            << 2;
    }

    #[inline]
    fn fetch_low_tile_byte(&mut self, cartridge: &mut dyn Cartridge, interrupt: &mut Interrupt) {
        self.next_tile.low_byte = self.read_vram(
            self.next_tile.tile_addr as usize,
            cartridge,
            interrupt,
            PpuReadAccess::BackgroundPattern,
        );
    }

    #[inline]
    fn fetch_high_tile_byte(&mut self, cartridge: &mut dyn Cartridge, interrupt: &mut Interrupt) {
        self.next_tile.high_byte = self.read_vram(
            self.next_tile.tile_addr as usize + 8,
            cartridge,
            interrupt,
            PpuReadAccess::BackgroundPattern,
        );
    }

    #[inline]
    fn fetch_tile(&mut self, cartridge: &mut dyn Cartridge, interrupt: &mut Interrupt) {
        if self.render_executing {
            match self.cycle & 7 {
                1 => self.fetch_name_table_byte(cartridge, interrupt),
                3 => self.fetch_attribute_table_byte(cartridge, interrupt),
                4 => self.fetch_low_tile_byte(cartridge, interrupt),
                6 => self.fetch_high_tile_byte(cartridge, interrupt),
                _ => {}
            }
        }
    }

    #[inline]
    fn tile_address(&self, tile: u8, offset: u16) -> usize {
        let tile = usize::from(tile);
        let offset = usize::from(offset);

        if self.control.sprite_size {
            ((tile & 1) * 0x1000)
                | (((tile & 0xFE) << 4) + (if offset >= 8 { offset + 8 } else { offset }))
        } else {
            ((tile << 4) | (if self.control.sprite_table { 0x1000 } else { 0 })) + offset
        }
    }

    #[inline]
    fn fetch_sprite_pattern(&mut self, cartridge: &mut dyn Cartridge, interrupt: &mut Interrupt) {
        let position_y = u16::from(self.secondary_oam[usize::from(self.sprite_index) << 2]);
        let tile = self.secondary_oam[(usize::from(self.sprite_index) << 2) + 1];
        let attribute = self.secondary_oam[(usize::from(self.sprite_index) << 2) + 2];
        let position_x = self.secondary_oam[(usize::from(self.sprite_index) << 2) + 3];

        let tile_address = if self.sprite_index < self.sprite_count
            && self.scan_line > position_y
            && self.scan_line <= position_y + (if self.control.sprite_size { 16 } else { 8 })
        {
            let line_offset = if attribute & 0x80 != 0 {
                (if self.control.sprite_size { 16 } else { 8 }) - (self.scan_line - position_y)
            } else {
                self.scan_line - position_y - 1
            };
            self.tile_address(tile, line_offset)
        } else {
            self.tile_address(0xFF, 0)
        };

        let read_address = if position_y < 240 {
            tile_address
        } else {
            self.tile_address(0xFF, 0)
        };

        let low_byte = self.read_vram(
            read_address,
            cartridge,
            interrupt,
            PpuReadAccess::SpritePattern,
        );
        let high_byte = self.read_vram(
            read_address + 8,
            cartridge,
            interrupt,
            PpuReadAccess::SpritePattern,
        );

        if (self.sprite_index < self.sprite_count) && position_y < 240 {
            let info = &mut self.sprites[usize::from(self.sprite_index)];
            info.priority = attribute & 0x20 != 0;
            info.horizontal_mirror = attribute & 0x40 != 0;
            info.palette_offset = ((attribute & 0x03) << 2) | 0x10;
            info.low_byte = low_byte;
            info.high_byte = high_byte;
            info.tile_addr = tile_address as u16;
            info.position = position_x;
            if self.scan_line > 0 {
                self.has_next_sprite = true;
            }
        }

        self.sprite_index += 1;
    }

    #[inline]
    fn show_background(&self) -> bool {
        (self.cycle > 300 || self.mask.show_background)
            && (self.cycle > 8 || self.mask.show_left_background)
    }

    #[inline]
    fn show_sprite(&self) -> bool {
        (self.cycle > 300 || self.mask.show_sprites)
            && (self.cycle > 8 || self.mask.show_left_sprites)
    }

    #[inline]
    fn background_pixel(&self) -> u8 {
        ((((self.state.low_bit_shift << self.state.x_scroll) & 0x8000) >> 15)
            | (((self.state.high_bit_shift << self.state.x_scroll) & 0x8000) >> 14)) as u8
    }

    #[inline]
    fn background_palette_index(palette_offset: u8, bg: u8) -> u8 {
        if bg == 0 { 0 } else { palette_offset + bg }
    }

    #[inline]
    fn evaluate_pixel(&mut self) -> u8 {
        let show_background =
            self.mask.show_background && (self.cycle > 8 || self.mask.show_left_background);
        let bg = if show_background {
            self.background_pixel()
        } else {
            0
        };

        let bg_tile = if u16::from(self.state.x_scroll) + ((self.cycle - 1) & 0x07) < 8 {
            self.previous_tile
        } else {
            self.current_tile
        };
        let bg_result = Self::background_palette_index(bg_tile.palette_offset, bg);

        let show_sprite = self.mask.show_sprites && (self.cycle > 8 || self.mask.show_left_sprites);
        if self.has_next_sprite && show_sprite {
            for i in 0..self.sprite_count {
                let s: &SpriteInfo = &self.sprites[usize::from(i)];
                if self.cycle > u16::from(s.position) {
                    let shift = self.cycle - u16::from(s.position) - 1;
                    if shift < 8 {
                        let sprite_color = if s.horizontal_mirror {
                            ((s.low_byte >> shift) & 0x01) | (((s.high_byte >> shift) & 0x01) << 1)
                        } else {
                            (((s.low_byte << shift) & 0x80) >> 7)
                                | (((s.high_byte << shift) & 0x80) >> 6)
                        };

                        if sprite_color != 0 {
                            if i == 0
                                && bg != 0
                                && self.has_first_sprite
                                && self.cycle != 256
                                && !self.status.sprite_zero_hit
                            {
                                self.status.sprite_zero_hit = true;
                            }
                            return if bg == 0 || !s.priority {
                                s.palette_offset + sprite_color
                            } else {
                                bg_result
                            };
                        }
                    }
                }
            }
        }
        bg_result
    }

    #[inline]
    fn render_pixel<S: Screen>(&mut self, screen: &mut S) {
        let color = if self.render_executing || ((self.state.vram_addr & 0x3F00) == 0) {
            usize::from(self.evaluate_pixel())
        } else {
            self.state.vram_addr as usize
        };
        screen.push(self.read_palette(color) & 0x3F);
        // self.screen_buffer[(self.cycle as usize - 1) + (self.scan_line as usize - 1) * 256]
    }

    #[inline]
    fn render_screen<S: Screen>(&mut self, screen: &mut S) {
        screen.render();
        // self.screen_buffer.iter().forEach(screen.push);
    }

    fn evaluate_sprites(&mut self) {
        if self.render_executing {
            match self.cycle {
                0..=64 => {
                    self.oam_read_buffer = 0xFF;
                    self.secondary_oam[usize::from(self.cycle - 1) >> 1] = 0xFF;
                    return;
                }
                65 => {
                    self.has_first_sprite_next = false;
                    self.has_sprite = false;
                    self.secondary_oam_address = 0;
                    self.sprite_overflow_delay = 0;

                    self.sprite_reading = true;
                    self.oam_address_high = (self.state.oam_address >> 2) & 0x3F;
                    self.oam_address_low = self.state.oam_address & 3;
                }
                256 => {
                    self.has_first_sprite = self.has_first_sprite_next;
                    self.sprite_count = self.secondary_oam_address >> 2;
                }
                _ => (),
            }
            if self.cycle & 1 != 0 {
                self.oam_read_buffer = self.primary_oam[usize::from(self.state.oam_address)];
            } else {
                if !self.sprite_reading {
                    self.oam_address_high = (self.oam_address_high + 1) & 0x3F;
                    if self.secondary_oam_address >= 0x20 {
                        self.oam_read_buffer =
                            self.secondary_oam[usize::from(self.secondary_oam_address) & 0x1F];
                    }
                } else {
                    if !self.has_sprite
                        && self.scan_line > u16::from(self.oam_read_buffer)
                        && self.scan_line
                            <= (u16::from(self.oam_read_buffer)
                                + if self.control.sprite_size { 16 } else { 8 })
                    {
                        self.has_sprite = true;
                    }

                    if self.secondary_oam_address < 0x20 {
                        self.secondary_oam[usize::from(self.secondary_oam_address)] =
                            self.oam_read_buffer;
                        if self.has_sprite {
                            self.oam_address_low += 1;
                            self.secondary_oam_address = self.secondary_oam_address.wrapping_add(1);

                            if self.oam_address_high == 0 {
                                self.has_first_sprite_next = true;
                            }

                            if self.oam_address_low == 4 {
                                self.has_sprite = false;
                                self.oam_address_low = 0;
                                self.oam_address_high = (self.oam_address_high + 1) & 0x3F;
                                if self.oam_address_high == 0 {
                                    self.sprite_reading = false;
                                }
                            }
                        } else {
                            self.oam_address_high = (self.oam_address_high + 1) & 0x3F;
                            if self.oam_address_high == 0 {
                                self.sprite_reading = false;
                            }
                        }
                    } else {
                        self.oam_read_buffer =
                            self.secondary_oam[usize::from(self.secondary_oam_address) & 0x1F];
                        if self.has_sprite {
                            self.status.sprite_overflow = true;
                            self.oam_address_low += 1;
                            if self.oam_address_low == 4 {
                                self.oam_address_low = 0;
                                self.oam_address_high = (self.oam_address_high + 1) & 0x3F;
                            }
                            if self.sprite_overflow_delay == 0 {
                                self.sprite_overflow_delay = 3;
                            } else if self.sprite_overflow_delay > 0 {
                                self.sprite_overflow_delay -= 1;
                                if self.sprite_overflow_delay == 0 {
                                    self.sprite_reading = false;
                                    self.oam_address_low = 0;
                                }
                            }
                        } else {
                            self.oam_address_high = (self.oam_address_high + 1) & 0x3F;
                            self.oam_address_low = (self.oam_address_low + 1) & 3;
                            if self.oam_address_high == 0 {
                                self.sprite_reading = false;
                            }
                        }
                    }
                }
                self.state.oam_address = (self.oam_address_high << 2) | (self.oam_address_low & 3);
            }
        }
    }

    #[inline]
    fn set_vertical_blank(&mut self, interrupt: &mut Interrupt) {
        self.status.nmi_occurred = true;
        if self.control.nmi_output {
            interrupt.nmi = true;
        }
    }

    pub(crate) fn step<S: Screen>(
        &mut self,
        screen: &mut S,
        cartridge: &mut dyn Cartridge,
        interrupt: &mut Interrupt,
    ) -> bool {
        let mut result = false;
        if self.cycle > 339 {
            self.cycle = 0;
            self.scan_line += 1;
            match self.scan_line {
                241 => {
                    self.frames += 1;
                    result = true;
                    self.openbus_io.next();
                    self.render_screen(screen);
                }
                NMI_SCAN_LINE => self.set_vertical_blank(interrupt),
                TOTAL_SCAN_LINE => {
                    self.status.sprite_overflow = false;
                    self.status.sprite_zero_hit = false;
                    self.scan_line = 0;
                }
                x @ 1..=TOTAL_SCAN_LINE
                    if x != 241 && x != NMI_SCAN_LINE && x != TOTAL_SCAN_LINE => {}
                _ => unreachable!(),
            }
        } else {
            self.bus_tick += 1;
            self.cycle += 1;
            if self.scan_line <= 240 {
                match self.cycle {
                    1..=256 => {
                        self.fetch_tile(cartridge, interrupt);
                        if self.post_render_executing && (self.cycle & 7) == 0 {
                            self.increment_x();
                            if self.cycle == 256 {
                                self.increment_y();
                            }
                        }

                        if self.scan_line > 0 {
                            self.render_pixel(screen);
                            self.state.low_bit_shift <<= 1;
                            self.state.high_bit_shift <<= 1;

                            self.evaluate_sprites();
                        } else if self.cycle < 9 {
                            if self.cycle == 1 {
                                self.status.nmi_occurred = false;
                            }
                            if self.state.oam_address >= 8 && self.render_executing {
                                self.primary_oam[usize::from(self.cycle) - 1] = self.primary_oam
                                    [usize::from(self.state.oam_address & 0xF8)
                                        + usize::from(self.cycle)
                                        - 1];
                            }
                        }
                    }
                    257..=320 => {
                        if self.cycle == 257 {
                            self.sprite_index = 0;
                            self.has_next_sprite = false;
                            if self.post_render_executing {
                                self.state.vram_addr = (self.state.vram_addr & !0x041F)
                                    | (self.state.temp_vram_addr & 0x041F);
                            }
                        }
                        if self.render_executing {
                            self.state.oam_address = 0;
                            match self.cycle & 7 {
                                1 => {
                                    let _ = self.read_vram(
                                        self.state.name_table_address(),
                                        cartridge,
                                        interrupt,
                                        PpuReadAccess::BackgroundNameTable,
                                    );
                                }
                                3 => {
                                    let _ = self.read_vram(
                                        self.state.attribute_address(),
                                        cartridge,
                                        interrupt,
                                        PpuReadAccess::BackgroundAttribute,
                                    );
                                }
                                4 => self.fetch_sprite_pattern(cartridge, interrupt),
                                _ => (),
                            }
                            if self.scan_line == 0 && self.cycle >= 280 && self.cycle <= 304 {
                                self.state.vram_addr = (self.state.vram_addr & !0x7BE0)
                                    | (self.state.temp_vram_addr & 0x7BE0);
                            }
                        }
                    }
                    321 => {
                        self.fetch_tile(cartridge, interrupt);
                        if self.render_executing {
                            self.oam_read_buffer = self.secondary_oam[0];
                        }
                    }
                    328 | 336 => {
                        self.fetch_tile(cartridge, interrupt);
                        if self.post_render_executing {
                            self.state.low_bit_shift <<= 8;
                            self.state.high_bit_shift <<= 8;
                            self.increment_x();
                        }
                    }
                    322..=327 | 329..=335 => {
                        self.fetch_tile(cartridge, interrupt);
                    }
                    337 => {
                        if self.render_executing {
                            let _ = self.read_vram(
                                self.state.name_table_address(),
                                cartridge,
                                interrupt,
                                PpuReadAccess::BackgroundNameTable,
                            );
                        }
                    }
                    338 => (),
                    339 => {
                        if self.render_executing {
                            let _ = self.read_vram(
                                self.state.name_table_address(),
                                cartridge,
                                interrupt,
                                PpuReadAccess::BackgroundNameTable,
                            );
                            if self.scan_line == 0 && (self.frames & 1) != 0 {
                                self.cycle += 1;
                            }
                        }
                    }
                    340 => (),
                    _ => unreachable!(),
                }
            }
        }

        self.post_render_executing = self.render_executing;
        self.render_executing = self.mask.show_background | self.mask.show_sprites;

        if self.vram_read_delay > 0 {
            self.vram_read_delay -= 1;
        }

        if self.vram_addr_update_delay > 0 {
            self.vram_addr_update_delay -= 1;
            let previous_vram_addr = self.state.vram_addr;
            self.state.vram_addr = self.new_vram_addr;
            let address = usize::from(self.state.vram_addr & 0x3FFF);
            if self.state.vram_addr != previous_vram_addr {
                cartridge.notify_ppu_bus_event(
                    PpuBusEvent::AddressBusUpdate {
                        address,
                        ppu_tick: self.ppu_bus_tick(),
                        from_cpu_register: true,
                        access: PpuBusAccess::Read,
                    },
                    interrupt,
                );
            }
        }
        result
    }
}

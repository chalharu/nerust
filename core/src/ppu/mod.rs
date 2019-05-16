// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod spriteinfo;
mod tileinfo;

use self::spriteinfo::SpriteInfo;
use self::tileinfo::TileInfo;
use crate::cartridge::Cartridge;
use crate::cpu::interrupt::Interrupt;
use crate::{OpenBus, OpenBusReadResult};
use nerust_screen_traits::Screen;
use std::cmp;
use std::mem;

const NMI_SCAN_LINE: u16 = 242;
const TOTAL_SCAN_LINE: u16 = 262;

#[derive(Serialize, Deserialize)]
struct DecayableOpenBus {
    data: u8,
    decay: [u8; 8],
}

impl DecayableOpenBus {
    pub fn new() -> Self {
        Self {
            data: 0,
            decay: [0; 8],
        }
    }

    pub fn unite(&mut self, data: OpenBusReadResult) -> u8 {
        for i in 0..8 {
            if (data.mask >> i) == 1 {
                self.decay[i] = 20;
            }
        }
        let result = (self.data & !data.mask) | (data.data & data.mask);
        self.data = result;
        result
    }

    pub fn write(&mut self, data: u8) -> u8 {
        self.data = data;
        self.decay = [20; 8];
        data
    }

    pub fn next(&mut self) {
        let mut result_mask: u8 = 0;
        for i in 0..8 {
            if self.decay[i] > 0 {
                self.decay[i] -= 1;
                result_mask |= 1 << i;
            }
        }
        self.data &= result_mask;
    }
}

#[derive(Serialize, Deserialize)]
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
    pub fn new() -> Self {
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

    pub fn reset(&mut self) {
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

#[derive(Serialize, Deserialize)]
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
    pub fn new() -> Self {
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

    pub fn reset(&mut self) {
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

#[derive(Serialize, Deserialize)]
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
    pub fn new() -> Self {
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

    pub fn reset(&mut self) {
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

#[derive(Serialize, Deserialize)]
struct Status {
    sprite_zero_hit: bool,
    sprite_overflow: bool,
    nmi_occurred: bool,
}

impl Status {
    pub fn new() -> Self {
        Self {
            sprite_zero_hit: false,
            sprite_overflow: false,
            nmi_occurred: false,
        }
    }

    pub fn reset(&mut self) {
        self.sprite_zero_hit = false;
        self.sprite_overflow = false;
        self.nmi_occurred = false;
    }
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Core {
    // memory
    #[serde(with = "nerust_serialize::BigArray")]
    vram: [u8; 2048],
    palette: [u8; 32],

    state: State,
    cycle: u16,
    scan_line: u16,
    frames: usize,
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
    pub fn new() -> Self {
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

    pub fn reset(&mut self) {
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

    pub fn read_register(
        &mut self,
        address: usize,
        cartridge: &mut Cartridge,
        interrupt: &mut Interrupt,
    ) -> OpenBusReadResult {
        let result = match address {
            0x2002 => OpenBusReadResult::new(self.read_status(interrupt), 0b1110_0000),
            0x2004 => OpenBusReadResult::new(self.read_oam(), 0xFF),
            0x2007 => self.read_data(cartridge),
            _ => OpenBusReadResult::new(0, 0),
        };
        OpenBusReadResult::new(self.openbus_io.unite(result), 0xFF)
    }

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

    fn read_data(&mut self, cartridge: &mut Cartridge) -> OpenBusReadResult {
        if self.vram_read_delay > 0 {
            OpenBusReadResult::new(0, 0)
        } else {
            self.vram_read_delay = 6;
            let addr = self.state.vram_addr as usize;
            let mut value = self.read_vram(addr, cartridge);
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
            self.increment_address(cartridge);
            OpenBusReadResult::new(value, mask)
        }
    }

    fn increment_address(&mut self, cartridge: &mut Cartridge) {
        if self.scan_line > 240 || !self.render_executing {
            self.state.vram_addr =
                (self.state.vram_addr + if self.control.increment { 32 } else { 1 }) & 0x7FFF;
            self.read_vram(self.state.vram_addr as usize, cartridge);
        } else {
            self.increment_x();
            self.increment_y();
        }
    }

    fn increment_x(&mut self) {
        if self.state.vram_addr & 0x1F == 0x1F {
            self.state.vram_addr = (self.state.vram_addr & 0xFFE0) ^ 0x0400;
        } else {
            self.state.vram_addr = self.state.vram_addr.wrapping_add(1);
        }
    }

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

    pub fn read_vram(&mut self, mut address: usize, cartridge: &mut Cartridge) -> u8 {
        address &= 0x3FFF;
        cartridge.vram_address_change(address);
        let result = match address {
            0...0x1FFF => cartridge.read(address),
            0x2000...0x3FFF => OpenBusReadResult::new(
                self.vram[cartridge.mirror_mode().mirror_address(address) & 0x7FF],
                0xFF,
            ),
            _ => {
                error!("unhandled ppu memory read at address: 0x{:04X}", address);
                OpenBusReadResult::new(0, 0)
            }
        };
        self.openbus_vram.unite(result)
    }

    fn palette_address(address: usize) -> usize {
        address & if address & 0x13 == 0x10 { 0x0F } else { 0x1F }
    }

    fn read_palette(&self, address: usize) -> u8 {
        self.palette[Self::palette_address(address)]
    }

    fn write_palette(&mut self, address: usize, value: u8) {
        self.palette[Self::palette_address(address)] = value & 0x3F;
    }

    pub fn write_register(
        &mut self,
        address: usize,
        value: u8,
        cartridge: &mut Cartridge,
        interrupt: &mut Interrupt,
    ) {
        match address {
            0x2000 => self.write_control(value, interrupt),
            0x2001 => self.write_mask(value),
            0x2003 => self.write_oam_address(value),
            0x2004 => self.write_oam_data(value),
            0x2005 => self.write_scroll(value),
            0x2006 => self.write_address(value),
            0x2007 => self.write_data(value, cartridge, interrupt),
            _ => {}
        }
        self.openbus_io.write(value);
    }

    fn write_control(&mut self, value: u8, interrupt: &mut Interrupt) {
        let prev_nmi_output = self.control.nmi_output;

        self.control = Control::from(value);

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

    fn write_mask(&mut self, value: u8) {
        self.mask = Mask::from(value);
    }

    fn write_oam_address(&mut self, value: u8) {
        self.state.oam_address = value;
    }

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

    fn write_data(&mut self, value: u8, cartridge: &mut Cartridge, interrupt: &mut Interrupt) {
        let addr = (self.state.vram_addr & 0x3FFF) as usize;

        if addr < 0x3F00 {
            self.write_vram(addr, value, cartridge, interrupt);
        } else {
            self.write_palette(addr, value);
        }
        self.increment_address(cartridge);
    }

    fn write_vram(
        &mut self,
        mut address: usize,
        value: u8,
        cartridge: &mut Cartridge,
        interrupt: &mut Interrupt,
    ) {
        address &= 0x3FFF;
        cartridge.vram_address_change(address);
        match address {
            0...0x1FFF => cartridge.write(address, value, interrupt),
            0x2000...0x3FFF => {
                self.vram[cartridge.mirror_mode().mirror_address(address) & 0x7FF] = value
            }
            _ => error!("unhandled ppu memory write at address: 0x{:04X}", address),
        }
    }

    fn fetch_name_table_byte(&mut self, cartridge: &mut Cartridge) {
        self.previous_tile = mem::replace(&mut self.current_tile, self.next_tile);
        self.state.low_bit_shift |= u16::from(self.next_tile.low_byte);
        self.state.high_bit_shift |= u16::from(self.next_tile.high_byte);
        self.next_tile.tile_addr =
            (u16::from(self.read_vram(self.state.name_table_address(), cartridge)) << 4)
                | ((self.state.vram_addr >> 12) & 7)
                | if self.control.background_table {
                    0x1000
                } else {
                    0
                };
    }

    fn fetch_attribute_table_byte(&mut self, cartridge: &mut Cartridge) {
        let v = self.state.vram_addr as usize;
        let address = self.state.attribute_address();
        let shift = ((v >> 4) & 4) | (v & 2);
        self.next_tile.palette_offset = ((self.read_vram(address, cartridge) >> shift) & 3) << 2;
    }

    fn fetch_low_tile_byte(&mut self, cartridge: &mut Cartridge) {
        self.next_tile.low_byte = self.read_vram(self.next_tile.tile_addr as usize, cartridge);
    }

    fn fetch_high_tile_byte(&mut self, cartridge: &mut Cartridge) {
        self.next_tile.high_byte = self.read_vram(self.next_tile.tile_addr as usize + 8, cartridge);
    }

    fn fetch_tile(&mut self, cartridge: &mut Cartridge) {
        if self.render_executing {
            match self.cycle & 7 {
                1 => self.fetch_name_table_byte(cartridge),
                3 => self.fetch_attribute_table_byte(cartridge),
                4 => self.fetch_low_tile_byte(cartridge),
                6 => self.fetch_high_tile_byte(cartridge),
                _ => {}
            }
        }
    }

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

    fn fetch_sprite_pattern(&mut self, cartridge: &mut Cartridge) {
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

        let low_byte = self.read_vram(read_address, cartridge);
        let high_byte = self.read_vram(read_address + 8, cartridge);

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

    fn show_background(&self) -> bool {
        (self.cycle > 300 || self.mask.show_background)
            && (self.cycle > 8 || self.mask.show_left_background)
    }

    fn show_sprite(&self) -> bool {
        (self.cycle > 300 || self.mask.show_sprites)
            && (self.cycle > 8 || self.mask.show_left_sprites)
    }

    fn background_pixel(&self) -> u8 {
        ((((self.state.low_bit_shift << self.state.x_scroll) & 0x8000) >> 15)
            | (((self.state.high_bit_shift << self.state.x_scroll) & 0x8000) >> 14)) as u8
    }

    fn evaluate_pixel(&mut self) -> u8 {
        let bg = if self.show_background() {
            self.background_pixel()
        } else {
            0
        };

        let bg_result_func = |s: &mut Self| {
            (if u16::from(s.state.x_scroll) + ((s.cycle - 1) & 0x07) < 8 {
                s.previous_tile
            } else {
                s.current_tile
            })
            .palette_offset
                + bg
        };

        if self.show_sprite() & self.has_next_sprite {
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
                                bg_result_func(self)
                            };
                        }
                    }
                }
            }
        }
        bg_result_func(self)
    }

    fn render_pixel<S: Screen>(&mut self, screen: &mut S) {
        let color = if self.render_executing || ((self.state.vram_addr & 0x3F00) == 0) {
            usize::from(self.evaluate_pixel())
        } else {
            self.state.vram_addr as usize
        };
        screen.push(self.read_palette(color) & 0x3F);
        // self.screen_buffer[(self.cycle as usize - 1) + (self.scan_line as usize - 1) * 256]
    }

    fn render_screen<S: Screen>(&mut self, screen: &mut S) {
        screen.render();
        // self.screen_buffer.iter().forEach(screen.push);
    }

    fn evaluate_sprites(&mut self) {
        if self.render_executing {
            match self.cycle {
                0...64 => {
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

    fn set_vertical_blank(&mut self, interrupt: &mut Interrupt) {
        self.status.nmi_occurred = true;
        if self.control.nmi_output {
            interrupt.nmi = true;
        }
    }

    pub(crate) fn step<S: Screen>(
        &mut self,
        screen: &mut S,
        cartridge: &mut Cartridge,
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
                1...TOTAL_SCAN_LINE => (),
                _ => unreachable!(),
            }
        } else {
            self.cycle += 1;
            if self.scan_line <= 240 {
                match self.cycle {
                    1...256 => {
                        self.fetch_tile(cartridge);
                        if self.post_render_executing && self.cycle.trailing_zeros() >= 3 {
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
                    257...320 => {
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
                                    self.read_vram(self.state.name_table_address(), cartridge);
                                }
                                3 => {
                                    self.read_vram(self.state.attribute_address(), cartridge);
                                }
                                4 => self.fetch_sprite_pattern(cartridge),
                                _ => (),
                            }
                            if self.scan_line == 0 && self.cycle >= 280 && self.cycle <= 304 {
                                self.state.vram_addr = (self.state.vram_addr & !0x7BE0)
                                    | (self.state.temp_vram_addr & 0x7BE0);
                            }
                        }
                    }
                    321 => {
                        self.fetch_tile(cartridge);
                        if self.render_executing {
                            self.oam_read_buffer = self.secondary_oam[0];
                        }
                    }
                    328 | 336 => {
                        self.fetch_tile(cartridge);
                        if self.post_render_executing {
                            self.state.low_bit_shift <<= 8;
                            self.state.high_bit_shift <<= 8;
                            self.increment_x();
                        }
                    }
                    322...327 | 329...335 => {
                        self.fetch_tile(cartridge);
                    }
                    337 => {
                        if self.render_executing {
                            self.read_vram(self.state.name_table_address(), cartridge);
                        }
                    }
                    338 => (),
                    339 => {
                        if self.render_executing {
                            self.read_vram(self.state.name_table_address(), cartridge);
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
            self.state.vram_addr = self.new_vram_addr;
            if self.scan_line > 240 || !self.render_executing {
                cartridge.vram_address_change(self.new_vram_addr as usize);
            }
        }
        result
    }
}

// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod memory;

use self::memory::Memory;
use nes::cartridge::Cartridge;
use nes::{Cpu, Screen, RGB};
use std::mem;
// use serde_bytes;

const PALETTE: [u32; 64] = [
    0x666666, 0x002A88, 0x1412A7, 0x3B00A4, 0x5C007E, 0x6E0040, 0x6C0600, 0x561D00, 0x333500,
    0x0B4800, 0x005200, 0x004F08, 0x00404D, 0x000000, 0x000000, 0x000000, 0xADADAD, 0x155FD9,
    0x4240FF, 0x7527FE, 0xA01ACC, 0xB71E7B, 0xB53120, 0x994E00, 0x6B6D00, 0x388700, 0x0C9300,
    0x008F32, 0x007C8D, 0x000000, 0x000000, 0x000000, 0xFFFEFF, 0x64B0FF, 0x9290FF, 0xC676FF,
    0xF36AFF, 0xFE6ECC, 0xFE8170, 0xEA9E22, 0xBCBE00, 0x88D800, 0x5CE430, 0x45E082, 0x48CDDE,
    0x4F4F4F, 0x000000, 0x000000, 0xFFFEFF, 0xC0DFFF, 0xD3D2FF, 0xE8C8FF, 0xFBC2FF, 0xFEC4EA,
    0xFECCC5, 0xF7D8A5, 0xE4E594, 0xCFEF96, 0xBDF4AB, 0xB3F3CC, 0xB5EBF2, 0xB8B8B8, 0x000000,
    0x000000,
];

// #[derive(Serialize, Deserialize, Debug)]
pub(crate) struct State {
    // #[serde(with = "serde_bytes")]
    vram: [u8; 2048],
    palette: [u8; 32],
    oam: [u8; 256],

    // ppuctrl
    name_table: u8,
    increment: bool,
    sprite_table: bool,
    background_table: bool,
    sprite_size: bool,
    master_slave: bool,
    //nmi_output: bool,

    // ppumask
    grayscale: bool,
    show_left_background: bool,
    show_left_sprites: bool,
    show_background: bool,
    show_sprites: bool,
    red_tint: bool,
    green_tint: bool,
    blue_tint: bool,

    // ppustatus
    sprite_zero_hit: bool,
    sprite_overflow: bool,

    oam_address: u8,

    // buffer
    buffered_data: u8,
    register: u8,

    // sprite buffer
    sprite_count: u8,
    sprite_patterns: [u32; 8],
    sprite_positions: [u8; 8],
    sprite_priorities: [bool; 8],
    sprite_indexes: [u8; 8],

    //
    v: u16,
    t: u16,
    x: u8,
    w: bool,
    f: bool,

    //
    cycle: u16,
    scan_line: u16,

    // NMI flags
    nmi_occurred: bool,
    nmi_output: bool,
    nmi_previous: bool,
    nmi_delay: u8,

    // background temporary variables
    name_table_byte: u8,
    attribute_table_byte: u8,
    low_tile_byte: u8,
    high_tile_byte: u8,
    tile_data: u64,

    register_buffer: u8,
}

impl State {
    pub fn new() -> Self {
        Self {
            vram: [0; 2048],
            palette: [0; 32],
            oam: [0; 256],
            name_table: 0,
            increment: false,
            sprite_table: false,
            background_table: false,
            sprite_size: false,
            master_slave: false,
            nmi_output: false,
            grayscale: false,
            show_left_background: false,
            show_left_sprites: false,
            show_background: false,
            show_sprites: false,
            red_tint: false,
            green_tint: false,
            blue_tint: false,
            sprite_zero_hit: false,
            sprite_overflow: false,
            oam_address: 0,
            buffered_data: 0,
            sprite_count: 0,
            sprite_patterns: [0; 8],
            sprite_positions: [0; 8],
            sprite_priorities: [false; 8],
            sprite_indexes: [0; 8],
            v: 0,
            t: 0,
            x: 0,
            w: false,
            f: false,
            register: 0,
            cycle: 340,
            scan_line: 240,
            nmi_occurred: false,
            nmi_previous: false,
            nmi_delay: 0,
            name_table_byte: 0,
            attribute_table_byte: 0,
            low_tile_byte: 0,
            high_tile_byte: 0,
            tile_data: 0,
            register_buffer: 0,
        }
    }

    fn increment_address(&mut self) {
        self.v += if self.increment { 32 } else { 1 };
    }

    fn increment_x(&mut self) {
        if self.v & 0x1F == 0x1F {
            self.v &= 0xFFE0;
            self.v ^= 0x0400;
        } else {
            self.v = self.v.wrapping_add(1);
        }
    }

    fn increment_y(&mut self) {
        if self.v & 0x7000 != 0x7000 {
            self.v = self.v.wrapping_add(0x1000);
        } else {
            self.v &= 0x8FFF;
            let x = match (self.v & 0x03E0) >> 5 {
                29 => {
                    self.v ^= 0x800;
                    0
                }
                31 => 0,
                y => y + 1,
            } << 5;
            self.v = (self.v & 0xFC1F) | x;
        }
    }

    fn copy_x(&mut self) {
        self.v = (self.v & 0xFBE0) | (self.t & 0x041F);
    }

    fn copy_y(&mut self) {
        self.v = (self.v & 0x841F) | (self.t & 0x7BE0);
    }

    fn nmi_change(&mut self) {
        let nmi = self.nmi_output && self.nmi_occurred;
        if nmi && !self.nmi_previous {
            // TODO: このdelayはよくわからない
            // self.nmi_delay = 15;
            self.nmi_delay = 1;
        }
        self.nmi_previous = nmi;
    }

    fn set_vertical_blank(&mut self) {
        self.nmi_occurred = true;
        self.nmi_change();
    }

    fn clear_vertical_blank(&mut self) {
        self.nmi_occurred = false;
        self.nmi_change();
    }

    fn read_status(&mut self) -> u8 {
        let result = self.register & 0x1F
            | if self.sprite_overflow { 0x20 } else { 0 }
            | if self.sprite_zero_hit { 0x40 } else { 0 }
            | if self.nmi_occurred { 0x80 } else { 0 };
        self.nmi_occurred = false;
        self.nmi_change();
        self.w = false;
        result
    }

    fn read_oam(&self) -> u8 {
        self.oam[usize::from(self.oam_address)]
    }

    fn write_control(&mut self, value: u8) {
        self.name_table = value & 3;
        self.increment = value & 4 != 0;
        self.sprite_table = value & 8 != 0;
        self.background_table = value & 0x10 != 0;
        self.sprite_size = value & 0x20 != 0;
        self.master_slave = value & 0x40 != 0;
        self.nmi_output = value & 0x80 != 0;
        self.nmi_change();

        // 画面の書き換え場所を変更
        self.t = (self.t & 0xF3FF) | (u16::from(value & 0x03) << 10);
    }

    fn write_mask(&mut self, value: u8) {
        self.grayscale = value & 1 != 0;
        self.show_left_background = value & 2 != 0;
        self.show_left_sprites = value & 4 != 0;
        self.show_background = value & 8 != 0;
        self.show_sprites = value & 0x10 != 0;
        self.red_tint = value & 0x20 != 0;
        self.green_tint = value & 0x40 != 0;
        self.blue_tint = value & 0x80 != 0;
    }

    fn write_oam_address(&mut self, value: u8) {
        self.oam_address = value;
    }

    fn write_oam_data(&mut self, value: u8) {
        self.oam[usize::from(self.oam_address)] = value;
        self.oam_address = self.oam_address.wrapping_add(1);
    }

    fn write_scroll(&mut self, value: u8) {
        if self.w {
            self.t = (self.t & 0x8FFF) | (u16::from(value & 0x07) << 12);
            self.t = (self.t & 0xFC1F) | (u16::from(value & 0xF8) << 2);
            self.w = false;
        } else {
            self.t = (self.t & 0xFFE0) | u16::from(value >> 3);
            self.x = value & 0x07;
            self.w = true;
        }
    }

    fn write_address(&mut self, value: u8) {
        if self.w {
            self.t = (self.t & 0xFF00) | u16::from(value);
            self.v = self.t;
            self.w = false;
        } else {
            self.t = (self.t & 0x80FF) | (u16::from(value & 0x3F) << 8);
            self.w = true;
        }
    }

    fn tile_address(&self) -> usize {
        (usize::from(self.name_table_byte) << 4)
            + ((self.v as usize >> 12) & 7)
            + if self.background_table { 0x1000 } else { 0 }
    }

    fn store_tile_data(&mut self) {
        self.tile_data |= u64::from(
            (0..8)
                .map(|i| {
                    u32::from(
                        self.attribute_table_byte
                            | ((self.low_tile_byte >> i) & 1)
                            | (((self.high_tile_byte >> i) & 1) << 1),
                    ) << (i << 2)
                }).sum::<u32>(),
        );
        // if self.attribute_table_byte != 0 || self.low_tile_byte != 0 || self.high_tile_byte != 0 {
        //     info!("store_tile_data: attribute_table_byte = 0x{:02X}, low_tile_byte = 0x{:02X}, high_tile_byte = 0x{:02X}, tile_data = 0x{:016X}", self.attribute_table_byte, self.low_tile_byte, self.high_tile_byte, self.tile_data);
        // }
    }

    fn fetch_tile_data(&self) -> u32 {
        (self.tile_data >> 32) as u32
    }

    fn background_pixel(&self) -> u8 {
        if self.show_background {
            (self.fetch_tile_data() >> ((7 - self.x) << 2)) as u8 & 0x0F
        } else {
            0
        }
    }

    fn sprite_pixel(&self) -> (u8, u8) {
        if self.show_sprites {
            self.sprite_positions
                .iter()
                .zip(self.sprite_patterns.iter())
                .enumerate()
                .take(usize::from(self.sprite_count))
                .filter(|(_, (&s, _))| u16::from(s) < self.cycle && self.cycle < u16::from(s) + 9)
                .map(|(i, (&s, &p))| {
                    (
                        i as u8,
                        (p >> ((8 + u16::from(s) - self.cycle) << 2)) as u8 & 0x0F,
                    )
                }).filter(|(_, c)| c & 3 != 0)
                .next()
                .unwrap_or_else(|| (0, 0))
        } else {
            (0, 0)
        }
    }

    fn render_pixel<S: Screen>(&mut self, screen: &mut S) {
        let bg = if self.cycle < 9 && !self.show_left_background {
            0
        } else {
            self.background_pixel()
        };
        let (i, sprite) = if self.cycle < 9 && !self.show_left_sprites {
            (0, 0)
        } else {
            self.sprite_pixel()
        };
        let color = match (bg & 3 != 0, sprite & 3 != 0) {
            (false, false) => 0,
            (false, true) => sprite | 0x10,
            (true, false) => bg,
            (true, true) => {
                if self.sprite_indexes[usize::from(i)] == 0 && self.cycle < 256 {
                    self.sprite_zero_hit = true;
                }
                if self.sprite_priorities[usize::from(i)] {
                    bg
                } else {
                    sprite | 0x10
                }
            }
        };
        screen.set_rgb(
            self.cycle - 1,
            self.scan_line,
            RGB::from(PALETTE[usize::from(self.read_palette(usize::from(color))) & 0x3F]),
        );
    }

    fn palette_address(address: usize) -> usize {
        address & if address & 0x13 == 0x10 { 0x0F } else { 0x1F }
    }

    fn read_palette(&self, address: usize) -> u8 {
        self.palette[Self::palette_address(address)]
    }

    fn write_palette(&mut self, address: usize, value: u8) {
        self.palette[Self::palette_address(address)] = value;
    }

    fn tick(&mut self) {
        if (self.show_background || self.show_sprites)
            && self.f
            && self.scan_line == 261
            && self.cycle == 339
        {
            self.cycle = 0;
            self.scan_line = 0;
            self.f = false;
        } else {
            self.cycle += 1;
            if self.cycle > 340 {
                self.cycle = 0;
                self.scan_line += 1;
                if self.scan_line > 261 {
                    self.scan_line = 0;
                    self.f = !self.f;
                }
            }
        }
    }
}

pub struct Core {
    state: State,
}

impl Core {
    pub fn new() -> Self {
        Self {
            state: State::new(),
        }
    }

    pub fn read_register(&mut self, address: usize, cartridge: &mut Box<Cartridge>) -> u8 {
        let result = match address {
            0x2002 => {
                (self.state.read_status() & 0b1110_0000)
                    | (self.state.register_buffer & 0b0001_1111)
            }
            0x2004 => self.state.read_oam(),
            0x2007 => self.read_data(cartridge),
            _ => self.state.register_buffer,
        };
        self.state.register_buffer = result;
        result
    }

    pub fn write_register(&mut self, address: usize, value: u8, cartridge: &mut Box<Cartridge>) {
        match address {
            0x2000 => self.state.write_control(value),
            0x2001 => self.state.write_mask(value),
            0x2003 => self.state.write_oam_address(value),
            0x2004 => self.state.write_oam_data(value),
            0x2005 => self.state.write_scroll(value),
            0x2006 => self.state.write_address(value),
            0x2007 => self.write_data(value, cartridge),
            _ => {}
        }
        self.state.register_buffer = value;
    }

    fn read_data(&mut self, cartridge: &mut Box<Cartridge>) -> u8 {
        let v = self.state.v;
        let mut value = Memory::new(&mut self.state, cartridge).read(v as usize);
        // emulate buffered reads
        if v & 0x3FFF < 0x3F00 {
            mem::swap(&mut self.state.buffered_data, &mut value);
        } else {
            self.state.buffered_data =
                Memory::new(&mut self.state, cartridge).read(v as usize - 0x1000);
        }
        self.state.increment_address();
        value
    }

    fn write_data(&mut self, value: u8, cartridge: &mut Box<Cartridge>) {
        let v = self.state.v;
        Memory::new(&mut self.state, cartridge).write(v as usize, value);
        self.state.increment_address();
    }

    pub fn write_dma(&mut self, value: &[u8]) {
        for &v in value {
            self.state.write_oam_data(v);
        }
    }

    fn fetch_name_table_byte(&mut self, cartridge: &mut Box<Cartridge>) {
        let address = 0x2000 | ((self.state.v as usize) & 0xFFF);
        self.state.name_table_byte = Memory::new(&mut self.state, cartridge).read(address);
    }

    fn fetch_attribute_table_byte(&mut self, cartridge: &mut Box<Cartridge>) {
        let v = self.state.v as usize;
        let address = 0x23C0 | (v & 0x0C00) | ((v >> 4) & 0x38) | ((v >> 2) & 0x07);
        let shift = ((v >> 4) & 4) | (v & 2);
        self.state.attribute_table_byte =
            ((Memory::new(&mut self.state, cartridge).read(address) >> shift) & 3) << 2;
    }

    fn fetch_low_tile_byte(&mut self, cartridge: &mut Box<Cartridge>) {
        let address = self.state.tile_address();
        self.state.low_tile_byte = Memory::new(&mut self.state, cartridge).read(address);
    }

    fn fetch_high_tile_byte(&mut self, cartridge: &mut Box<Cartridge>) {
        let address = self.state.tile_address() + 8;
        self.state.high_tile_byte = Memory::new(&mut self.state, cartridge).read(address);
    }

    fn fetch_sprite_pattern(
        &mut self,
        i: usize,
        row: usize,
        cartridge: &mut Box<Cartridge>,
    ) -> u32 {
        let tile = usize::from(self.state.oam[(i << 2) + 1]);
        let attributes = self.state.oam[(i << 2) + 2];
        let address = if self.state.sprite_size {
            (if attributes & 0x80 == 0x80 {
                ((15 - row) & 7) | ((8 - (row & 8)) << 1)
            } else {
                (row & 7) | ((row & 8) << 1)
            }) + ((tile & 1) * 0x1000)
                + ((tile & 0xFE) << 4)
        } else {
            (if self.state.sprite_table { 0x1000 } else { 0 })
                + (tile << 4)
                + (if attributes & 0x80 == 0x80 {
                    7 - row
                } else {
                    row
                })
        };
        let a = (attributes & 3) << 2;
        let memory = Memory::new(&mut self.state, cartridge);
        let low_tile_byte = memory.read(address);
        let high_tile_byte = memory.read(address + 8);
        (0..8)
            .map(|j| {
                u32::from(
                    a | if attributes & 0x40 == 0x40 {
                        (((low_tile_byte << j) & 0x80) >> 7) | (((high_tile_byte << j) & 0x80) >> 6)
                    } else {
                        ((low_tile_byte >> j) & 1) | (((high_tile_byte >> j) & 1) << 1)
                    },
                ) << (j << 2)
            }).sum::<u32>()
    }

    fn evaluate_sprites(&mut self, cartridge: &mut Box<Cartridge>) {
        let h = if self.state.sprite_size { 16 } else { 8 };
        let scan_line = self.state.scan_line as usize;
        let mut count = 0;
        for i in 0..64 {
            let y = usize::from(self.state.oam[i << 2]);
            let a = self.state.oam[(i << 2) + 2];
            let x = self.state.oam[(i << 2) + 3];
            if scan_line < y || scan_line >= h + y {
                continue;
            }

            if count < 8 {
                self.state.sprite_patterns[count] =
                    self.fetch_sprite_pattern(i, scan_line - y, cartridge);
                self.state.sprite_positions[count] = x;
                self.state.sprite_priorities[count] = (a >> 5) & 1 == 1;
                self.state.sprite_indexes[count] = i as u8;
            }
            count += 1;
        }
        if count > 8 {
            count = 8;
            self.state.sprite_overflow = true;
        }
        self.state.sprite_count = count as u8;
    }

    pub(crate) fn step<S: Screen>(
        &mut self,
        screen: &mut S,
        cartridge: &mut Box<Cartridge>,
        cpu: &mut Cpu,
    ) -> bool {
        if self.state.nmi_delay > 0 {
            self.state.nmi_delay -= 1;
            if self.state.nmi_delay == 0 && self.state.nmi_output && self.state.nmi_occurred {
                cpu.trigger_nmi();
            }
        };
        self.state.tick();
        let rendering_enabled = self.state.show_background || self.state.show_sprites;
        let pre_line = self.state.scan_line == 261;
        let visible_line = self.state.scan_line < 240;
        let render_line = pre_line || visible_line;
        let pre_fetch_cycle = self.state.cycle >= 321 && self.state.cycle <= 336;
        let visible_cycle = self.state.cycle >= 1 && self.state.cycle <= 256;
        let fetch_cycle = pre_fetch_cycle || visible_cycle;
        if rendering_enabled {
            if visible_line && visible_cycle {
                self.state.render_pixel(screen);
            }
            if render_line && fetch_cycle {
                self.state.tile_data <<= 4;
                match self.state.cycle & 7 {
                    1 => self.fetch_name_table_byte(cartridge),
                    3 => self.fetch_attribute_table_byte(cartridge),
                    5 => self.fetch_low_tile_byte(cartridge),
                    7 => self.fetch_high_tile_byte(cartridge),
                    0 => self.state.store_tile_data(),
                    _ => {}
                }
            }
            if pre_line && self.state.cycle >= 280 && self.state.cycle <= 304 {
                self.state.copy_y()
            }
            if render_line {
                if fetch_cycle && self.state.cycle % 8 == 0 {
                    self.state.increment_x()
                }
                if self.state.cycle == 256 {
                    self.state.increment_y()
                }
                if self.state.cycle == 257 {
                    self.state.copy_x()
                }
            }
            if self.state.cycle == 257 {
                if visible_line {
                    self.evaluate_sprites(cartridge);
                } else {
                    self.state.sprite_count = 0;
                }
            }
        }
        if pre_line && self.state.cycle == 1 {
            self.state.clear_vertical_blank();
            self.state.sprite_zero_hit = false;
            self.state.sprite_overflow = false;
        }
        if self.state.scan_line == 241 && self.state.cycle == 1 {
            self.state.set_vertical_blank();
            true
        } else {
            false
        }
    }
}

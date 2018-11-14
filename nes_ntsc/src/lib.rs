// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#[macro_use]
extern crate lazy_static;

#[macro_use]
extern crate log;

mod init;
mod setup;

pub use self::setup::Setup;
use self::setup::SetupValues;

use self::init::Init;
use std::mem;

const NES_NTSC_PALETTE_SIZE: usize = 64;
const NES_NTSC_ENTRY_SIZE: usize = 128;

const ALIGNMENT_COUNT: usize = 3;
const BURST_COUNT: usize = 3;
const RESCALE_IN: usize = 8;
const RESCALE_OUT: usize = 7;

const ARTIFACTS_MID: f32 = 1.0;
const ARTIFACTS_MAX: f32 = ARTIFACTS_MID * 1.5;

const FRINGING_MID: f32 = 1.0;
const FRINGING_MAX: f32 = FRINGING_MID * 2.0;

const STD_DECODER_HUE: f32 = -15.0;
// const EXT_DECODER_HUE: f32 = STD_DECODER_HUE + 15.0;
const LUMA_CUTOFF: f32 = 0.20;

const KERNEL_HALF: usize = 16;
const KERNEL_SIZE: usize = KERNEL_HALF * 2 + 1;

// const GAMMA_SIZE: usize = 1;

const LOW_LEVELS: [f32; 4] = [-0.12, 0.00, 0.31, 0.72];
const HIGH_LEVELS: [f32; 4] = [0.40, 0.68, 1.00, 1.00];

const DEFAULT_DECODER: [f32; 6] = [0.956, 0.621, -0.272, -0.647, -1.105, 1.702];

const RGB_BITS: usize = 8;
const RGB_UNIT: usize = 1 << RGB_BITS;
const RGB_OFFSET: f32 = RGB_UNIT as f32 * 2.0 + 0.5;
const BURST_SIZE: usize = NES_NTSC_ENTRY_SIZE / BURST_COUNT;
const RGB_KERNEL_SIZE: usize = BURST_SIZE / ALIGNMENT_COUNT;

const NES_NTSC_RGB_BUILDER: u32 = (1 << 21) | (1 << 11) | (1 << 1);
const RGB_BIAS: u32 = RGB_UNIT as u32 * 2 * NES_NTSC_RGB_BUILDER;

const NES_NTSC_IN_CHUNK: usize = 3;
const NES_NTSC_OUT_CHUNK: usize = 7;
const NES_NTSC_BURST_COUNT: usize = 3;
const NES_NTSC_BLACK: u8 = 15;
const NES_NTSC_BURST_SIZE: usize = NES_NTSC_ENTRY_SIZE / NES_NTSC_BURST_COUNT;
// const NES_NTSC_OUT_DEPTH: usize = 24;

const NES_NTSC_CLAMP_MASK: u32 = NES_NTSC_RGB_BUILDER * 3 / 2;
const NES_NTSC_CLAMP_ADD: u32 = NES_NTSC_RGB_BUILDER * 0x101;

struct PixelInfo {
    offset: usize,
    negate: f32,
    kernel: [f32; 4],
}

fn pixel_offset_impl(ntsc: isize, scaled: isize) -> usize {
    (KERNEL_SIZE as isize / 2
        + ntsc
        + if scaled != 0 { 1 } else { 0 }
        + (RESCALE_OUT as isize - scaled) % RESCALE_OUT as isize
        + (KERNEL_SIZE as isize * 2 * scaled)) as usize
}

fn pixel_offset(ntsc: isize, scaled: isize, kernel: [f32; 4]) -> PixelInfo {
    let offset = pixel_offset_impl(
        ntsc - scaled / RESCALE_OUT as isize * RESCALE_IN as isize,
        (scaled + RESCALE_OUT as isize * 10) % RESCALE_OUT as isize,
    );
    PixelInfo {
        offset,
        negate: (1.0 - ((ntsc + 100) & 2) as f32),
        kernel: kernel,
    }
}

// 3 input pixels -> 8 composite samples
lazy_static! {
    static ref NES_NTSC_PIXELS: [PixelInfo; ALIGNMENT_COUNT] = [
        pixel_offset(-4, -9, [1.0, 1.0, 0.6667, 0.0]),
        pixel_offset(-2, -7, [0.3333, 1.0, 1.0, 0.3333]),
        pixel_offset(0, -5, [0.0, 0.6667, 1.0, 1.0]),
    ];
}

fn rotate_iq(iq: &(f32, f32), sin_b: f32, cos_b: f32) -> (f32, f32) {
    (iq.0 * cos_b - iq.1 * sin_b, iq.0 * sin_b + iq.1 * cos_b)
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct RGB {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

impl From<u32> for RGB {
    fn from(value: u32) -> RGB {
        RGB {
            red: ((value >> 16) & 0xFF) as u8,
            green: ((value >> 8) & 0xFF) as u8,
            blue: (value & 0xFF) as u8,
        }
    }
}

// pub trait NesNtscExt<I1: IntoIterator<Item = u8>, I2: Iterator<Item = I1>> {
//     fn filter(self, kernel: &NesNtsc) -> NesNtscIterator<I1, I2>;
// }

// impl<I1, I2> NesNtscExt<I1, I2::IntoIter> for I2
// where
//     I1: IntoIterator<Item = u8>,
//     I2: IntoIterator<Item = I1>,
// {
//     fn filter(self, kernel: &NesNtsc) -> NesNtscIterator<I1, I2::IntoIter> {
//         NesNtscIterator::new(self.into_iter(), kernel)
//     }
// }

pub struct NesNtsc {
    burst: usize,
    table: Vec<u32>,
    width: usize,
    in_chunk_count: usize,
    row_pos: usize,
    row: Option<NtscRow>,
    chunk_size: usize,
}

impl NesNtsc {
    // phases [i] = cos( i * PI / 6 )
    const PHASES: [f32; 19] = [
        -1.0, -0.866025, -0.5, 0.0, 0.5, 0.866025, 1.0, 0.866025, 0.5, 0.0, -0.5, -0.866025, -1.0,
        -0.866025, -0.5, 0.0, 0.5, 0.866025, 1.0,
    ];

    fn to_angle_sin(color: usize) -> f32 {
        Self::PHASES[color]
    }

    fn to_angle_cos(color: usize) -> f32 {
        Self::PHASES[color + 3]
    }

    fn yiq_to_rgb_f32(y: f32, i: f32, q: f32, to_rgb: &[f32]) -> (f32, f32, f32) {
        (
            y + to_rgb[0] * i + to_rgb[1] * q,
            y + to_rgb[2] * i + to_rgb[3] * q,
            y + to_rgb[4] * i + to_rgb[5] * q,
        )
    }

    fn rgb_to_yiq_f32(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
        (
            r * 0.299 + g * 0.587 + b * 0.114,
            r * 0.596 - g * 0.275 - b * 0.321,
            r * 0.212 - g * 0.523 + b * 0.311,
        )
    }

    fn pack_rgb(r: u32, g: u32, b: u32) -> u32 {
        (r << 21) | (g << 11) | (b << 1)
    }

    // Generate pixel at all burst phases and column alignments
    fn gen_kernel(filter_impl: &Init, y: f32, mut i: f32, mut q: f32) -> Vec<u32> {
        // generate for each scanline burst phase

        // 	float const* to_rgb = impl->to_rgb;
        let y = y - RGB_OFFSET;
        let mut out = Vec::new();
        for k in 0..BURST_COUNT {
            // Encode yiq into *two* composite signals (to allow control over artifacting).
            // Convolve these with kernels which: filter respective components, apply
            // sharpening, and rescale horizontally. Convert resulting yiq to rgb and pack
            // into integer. Based on algorithm by NewRisingSun.

            // pixel_info_t const* pixel = nes_ntsc_pixels;
            for j in 0..ALIGNMENT_COUNT {
                // negate is -1 when composite starts at odd multiple of 2
                let pixel = &NES_NTSC_PIXELS[j];
                let yy = y * filter_impl.fringing * pixel.negate;
                let ic0 = (i + yy) * pixel.kernel[0];
                let qc1 = (q + yy) * pixel.kernel[1];
                let ic2 = (i - yy) * pixel.kernel[2];
                let qc3 = (q - yy) * pixel.kernel[3];

                let factor = filter_impl.artifacts * pixel.negate;
                let ii = i * factor;
                let yc0 = (y + ii) * pixel.kernel[0];
                let yc2 = (y - ii) * pixel.kernel[2];

                let qq = q * factor;
                let yc1 = (y + qq) * pixel.kernel[1];
                let yc3 = (y - qq) * pixel.kernel[3];

                let mut offset = pixel.offset;

                for _ in 0..RGB_KERNEL_SIZE {
                    let i =
                        filter_impl.kernel[offset + 0] * ic0 + filter_impl.kernel[offset + 2] * ic2;
                    let q =
                        filter_impl.kernel[offset + 1] * qc1 + filter_impl.kernel[offset + 3] * qc3;
                    let y = filter_impl.kernel[offset + KERNEL_SIZE + 0] * yc0
                        + filter_impl.kernel[offset + KERNEL_SIZE + 1] * yc1
                        + filter_impl.kernel[offset + KERNEL_SIZE + 2] * yc2
                        + filter_impl.kernel[offset + KERNEL_SIZE + 3] * yc3
                        + RGB_OFFSET;

                    if RESCALE_OUT <= 1 {
                        offset -= 1;
                    } else if offset < KERNEL_SIZE * 2 * (RESCALE_OUT - 1) {
                        offset += KERNEL_SIZE * 2 - 1;
                    } else {
                        offset -= KERNEL_SIZE * 2 * (RESCALE_OUT - 1) + 2;
                    }

                    let (r, g, b) = Self::yiq_to_rgb_f32(y, i, q, &filter_impl.to_rgb[k]);
                    out.push(Self::pack_rgb(r as u32, g as u32, b as u32).wrapping_sub(RGB_BIAS));
                }
            }
            let iq = rotate_iq(&(i, q), -0.866025, -0.5);
            i = iq.0;
            q = iq.1;
        }
        for _ in out.len()..NES_NTSC_ENTRY_SIZE {
            out.push(0)
        }
        out
    }

    fn merge_kernel_fields(io: &mut [u32]) {
        for i in 0..BURST_SIZE {
            let p0 = io[i + BURST_SIZE * 0].wrapping_add(RGB_BIAS);
            let p1 = io[i + BURST_SIZE * 1].wrapping_add(RGB_BIAS);
            let p2 = io[i + BURST_SIZE * 2].wrapping_add(RGB_BIAS);
            // merge colors without losing precision
            io[i + BURST_SIZE * 0] =
                ((p0 + p1 - ((p0 ^ p1) & NES_NTSC_RGB_BUILDER)) >> 1).wrapping_sub(RGB_BIAS);
            io[i + BURST_SIZE * 1] =
                ((p1 + p2 - ((p1 ^ p2) & NES_NTSC_RGB_BUILDER)) >> 1).wrapping_sub(RGB_BIAS);
            io[i + BURST_SIZE * 2] =
                ((p2 + p0 - ((p2 ^ p0) & NES_NTSC_RGB_BUILDER)) >> 1).wrapping_sub(RGB_BIAS);
        }
    }

    fn correct_errors(color: u32, kernel: &mut [u32]) {
        for j in 0..BURST_COUNT {
            let offset = j * ALIGNMENT_COUNT * RGB_KERNEL_SIZE;
            let out = &mut kernel[offset..];
            for i in 0..(RGB_KERNEL_SIZE / 2) {
                let error = color
                    .wrapping_sub(out[i])
                    .wrapping_sub(out[(i + 12) % 14 + 14])
                    .wrapping_sub(out[(i + 10) % 14 + 28])
                    .wrapping_sub(out[i + 7])
                    .wrapping_sub(out[i + 5 + 14])
                    .wrapping_sub(out[i + 3 + 28]);
                Self::distribute_error(i + 3 + 28, i + 5 + 14, i + 7, i, error, out);
            }
        }
    }

    fn distribute_error(a: usize, b: usize, c: usize, i: usize, error: u32, out: &mut [u32]) {
        let fourth = (((error + 2 * NES_NTSC_RGB_BUILDER) >> 2)
            & ((RGB_BIAS >> 1).wrapping_sub(NES_NTSC_RGB_BUILDER))).wrapping_sub(RGB_BIAS >> 2);
        out[a] = out[a].wrapping_add(fourth);
        out[b] = out[b].wrapping_add(fourth);
        out[c] = out[c].wrapping_add(fourth);
        out[i] = out[i]
            .wrapping_add(error)
            .wrapping_sub(fourth.wrapping_mul(3));
    }

    pub fn new(setup: &Setup, width: usize) -> Self {
        let filter_impl = Init::new(setup);

        // setup fast gamma
        let gamma_factor = {
            let gamma = setup.gamma() * -0.5 + 0.1333;
            gamma.abs().powf(0.73).abs()
        };

        let merge_fields =
            (setup.artifacts() <= -1.0 && setup.fringing() <= -1.0) || setup.merge_fields();

        let chunk_size = (width - 1) / NES_NTSC_IN_CHUNK;

        Self {
            chunk_size,
            width,
            burst: 0,
            in_chunk_count: 0,
            row_pos: 0,
            row: None,
            table: (0..NES_NTSC_PALETTE_SIZE)
                .map(|entry| {
                    // Base 64-color generation
                    let level = (entry >> 4) & 3;

                    let color = entry & 0x0F;
                    let (low, high) = match color {
                        0 => (HIGH_LEVELS[level], HIGH_LEVELS[level]),
                        0x0D => (LOW_LEVELS[level], LOW_LEVELS[level]),
                        0x0E | 0x0F => (0.0, 0.0),
                        _ => (LOW_LEVELS[level], HIGH_LEVELS[level]),
                    };

                    {
                        let (y, i, q) = {
                            // Convert raw waveform to YIQ
                            let sat = (high - low) * 0.5;
                            let i = Self::to_angle_sin(color) * sat;
                            let q = Self::to_angle_cos(color) * sat;
                            let y = ((high + low) * 0.5)
                    // Apply brightness, contrast, and gamma
                    * (setup.contrast() * 0.5 + 1.0)
                    // adjustment reduces error when using input palette
                    + setup.brightness() * 0.5
                                - 0.5 / 256.0;

                            let (r, g, b) = Self::yiq_to_rgb_f32(y, i, q, &DEFAULT_DECODER);
                            // fast approximation of n = pow( n, gamma )
                            let (y, i, q) = Self::rgb_to_yiq_f32(
                                (r * gamma_factor - gamma_factor) * r + r,
                                (g * gamma_factor - gamma_factor) * g + g,
                                (b * gamma_factor - gamma_factor) * b + b,
                            );
                            (
                                y * RGB_UNIT as f32 + RGB_OFFSET,
                                i * RGB_UNIT as f32,
                                q * RGB_UNIT as f32,
                            )
                        };

                        // Generate kernel
                        {
                            let (r, g, b) = Self::yiq_to_rgb_f32(y, i, q, &filter_impl.to_rgb[0]);
                            // blue tends to overflow, so clamp it
                            let rgb = Self::pack_rgb(
                                r as u32,
                                g as u32,
                                if (b as u32) < 0x3E0 { b as u32 } else { 0x3E0 },
                            );

                            let mut kernel = Self::gen_kernel(&filter_impl, y, i, q);
                            if merge_fields {
                                Self::merge_kernel_fields(&mut kernel);
                            }
                            Self::correct_errors(rgb, &mut kernel);
                            kernel
                        }
                    }
                }).flatten()
                .collect::<Vec<_>>(),
        }
    }

    pub fn set_burst(&mut self, burst: usize) {
        self.burst = burst;
    }

    pub fn set_source_width(&mut self, width: usize) {
        self.width = width;
        self.chunk_size = (width - 1) / NES_NTSC_IN_CHUNK;
    }

    pub fn output_width(source: usize) -> usize {
        ((source - 1) / NES_NTSC_IN_CHUNK + 1) * NES_NTSC_OUT_CHUNK
    }

    fn rgb_out<F: FnMut(RGB)>(&mut self, in_chunk: usize, value: u8, next_func: &mut F) {
        match in_chunk {
            0 => {
                self.row.as_mut().unwrap().color_in(0, value);
                next_func(self.row.as_ref().unwrap().rgb_out(&self.table, 0));
                next_func(self.row.as_ref().unwrap().rgb_out(&self.table, 1));
            }
            1 => {
                self.row.as_mut().unwrap().color_in(1, value);
                next_func(self.row.as_ref().unwrap().rgb_out(&self.table, 2));
                next_func(self.row.as_ref().unwrap().rgb_out(&self.table, 3));
            }
            2 => {
                self.row.as_mut().unwrap().color_in(2, value);
                next_func(self.row.as_ref().unwrap().rgb_out(&self.table, 4));
                next_func(self.row.as_ref().unwrap().rgb_out(&self.table, 5));
                next_func(self.row.as_ref().unwrap().rgb_out(&self.table, 6));
            }
            _ => unreachable!(),
        }
    }

    pub fn push<F: FnMut(RGB)>(&mut self, value: u8, next_func: &mut F) {
        if self.row_pos == 0 {
            mem::replace(
                &mut self.row,
                Some(NtscRow::new(
                    self.burst,
                    NES_NTSC_BLACK,
                    NES_NTSC_BLACK,
                    value,
                )),
            );
        } else {
            let chunk_count = self.in_chunk_count;
            self.rgb_out(chunk_count, value, next_func);
            self.in_chunk_count = match self.in_chunk_count {
                0 => 1,
                1 => 2,
                2 => 0,
                _ => unreachable!(),
            };
        }
        self.row_pos += 1;
        if self.row_pos == self.width {
            self.row_pos = 0;
            match self.in_chunk_count {
                0 => (),
                1 => {
                    self.rgb_out(1, NES_NTSC_BLACK, next_func);
                    self.rgb_out(2, NES_NTSC_BLACK, next_func);
                }
                2 => self.rgb_out(2, NES_NTSC_BLACK, next_func),
                _ => unreachable!(),
            };
            self.rgb_out(0, NES_NTSC_BLACK, next_func);
            self.rgb_out(1, NES_NTSC_BLACK, next_func);
            self.rgb_out(2, NES_NTSC_BLACK, next_func);
            self.burst = (self.burst + 1) % NES_NTSC_BURST_COUNT;
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
struct NtscRow {
    // burst: usize,
    // pixel: [u32; 3],
    kernel: [usize; 3],
    kernelx: [usize; 3],
    ktable_offset: usize,
}

impl NtscRow {
    // NES_NTSC_BEGIN_ROW
    pub(crate) fn new(burst: usize, pixel0: u8, pixel1: u8, pixel2: u8) -> Self {
        let ktable_offset = burst * NES_NTSC_BURST_SIZE;
        let kernel0 = Self::entry_impl(ktable_offset, pixel0);
        Self {
            // burst,
            kernelx: [0, kernel0, kernel0],
            kernel: [
                kernel0,
                Self::entry_impl(ktable_offset, pixel1),
                Self::entry_impl(ktable_offset, pixel2),
            ],
            ktable_offset,
            // pixel: [pixel0, pixel1, pixel2],
        }
    }

    // NES_NTSC_ENTRY_
    fn entry_impl(ktable_offset: usize, n: u8) -> usize {
        ktable_offset + usize::from(n) * NES_NTSC_ENTRY_SIZE
    }

    pub fn rgb_out(&self, input: &[u32], x: usize) -> RGB {
        RGB::from(Self::rgb_out_impl(Self::clamp_impl(
            input[self.kernel[0] + x]
                .wrapping_add(input[self.kernel[1] + (x + 12) % 7 + 14])
                .wrapping_add(input[self.kernel[2] + (x + 10) % 7 + 28])
                .wrapping_add(input[self.kernelx[0] + (x + 7) % 14])
                .wrapping_add(input[self.kernelx[1] + (x + 5) % 7 + 21])
                .wrapping_add(input[self.kernelx[2] + (x + 3) % 7 + 35]),
        )))
    }

    // common ntsc macros
    fn clamp_impl(io: u32) -> u32 {
        let sub = io >> 9 & NES_NTSC_CLAMP_MASK;
        let clamp = NES_NTSC_CLAMP_ADD - sub;
        (io | clamp) & (clamp - sub)
    }

    fn rgb_out_impl(raw: u32) -> u32 {
        ((raw >> 5) & 0xFF0000) | ((raw >> 3) & 0xFF00) | ((raw >> 1) & 0xFF)
    }

    // NES_NTSC_COLOR_IN
    pub fn color_in(&mut self, in_index: usize, color_in: u8) {
        self.kernelx[in_index] = self.kernel[in_index];
        self.kernel[in_index] = Self::entry_impl(self.ktable_offset, color_in);
    }
}

mod backdrop;
mod bg1;
mod color;
mod mode7;
mod obj;
mod tile;

use backdrop::render_presented_backdrop;
use bg1::render_bg1;
use color::{apply_color_math, opaque_black_screen, snes_color_to_rgba};
use nerust_snes_core::Core;
use obj::render_obj;

pub const SCREEN_WIDTH: usize = 256;
pub const SCREEN_HEIGHT: usize = 224;

/// Sentinel value for "no pixel" in raw output buffers.
/// 0xFFFF (bit 15 set) is not a valid 15-bit SNES color, so
/// it safely distinguishes "transparent/no pixel" from CGRAM[0]=0x0000 (black).
const TRANSPARENT: u16 = 0xFFFF;

pub const MODE5_6_WIDTH: usize = 512;
pub const INTERLACE_HEIGHT: usize = 448;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BgLayer {
    Bg1,
    Bg2,
    Bg3,
    Bg4,
}

impl BgLayer {
    const fn tm_mask(self) -> u8 {
        match self {
            Self::Bg1 => 0x01,
            Self::Bg2 => 0x02,
            Self::Bg3 => 0x04,
            Self::Bg4 => 0x08,
        }
    }

    const fn bit_index(self) -> usize {
        match self {
            Self::Bg1 => 0,
            Self::Bg2 => 1,
            Self::Bg3 => 2,
            Self::Bg4 => 3,
        }
    }

    const fn scroll_targets(self) -> (u8, u8) {
        match self {
            Self::Bg1 => (0x0D, 0x0E),
            Self::Bg2 => (0x0F, 0x10),
            Self::Bg3 => (0x11, 0x12),
            Self::Bg4 => (0x13, 0x14),
        }
    }
}

pub(crate) fn use_presented_main_screen(core: &Core) -> bool {
    if !hdma_targets_bbus(core, &[0x2C]) {
        return false;
    }

    let mut first = None;
    for line in 0..SCREEN_HEIGHT {
        let Some(screen) = core.presented_main_screen_line(line) else {
            continue;
        };
        let Some(first_screen) = first else {
            first = Some(screen);
            continue;
        };
        if screen != first_screen {
            return true;
        }
    }
    false
}

pub(crate) fn main_screen_for_line(
    core: &Core,
    screen_y: usize,
    current_tm: u8,
    use_presented_tm: bool,
) -> u8 {
    if use_presented_tm {
        core.presented_main_screen_line(screen_y)
            .map_or(current_tm, |line| line.tm)
    } else {
        current_tm
    }
}

pub(crate) fn use_presented_bg_scroll(core: &Core, layer: BgLayer) -> bool {
    let (hofs, vofs) = layer.scroll_targets();
    if !hdma_targets_bbus(core, &[hofs, vofs]) {
        return false;
    }

    let mut first = None;
    for line in 0..SCREEN_HEIGHT {
        let Some(scroll) = presented_bg_line(core, layer, line) else {
            continue;
        };
        let Some(first_scroll) = first else {
            first = Some(scroll);
            continue;
        };
        if scroll != first_scroll {
            return true;
        }
    }
    false
}

pub(crate) fn presented_bg_line(
    core: &Core,
    layer: BgLayer,
    screen_y: usize,
) -> Option<nerust_snes_core::PresentedBg1Line> {
    match layer {
        BgLayer::Bg1 => core.presented_bg1_line(screen_y),
        BgLayer::Bg2 => core.presented_bg2_line(screen_y),
        BgLayer::Bg3 => core.presented_bg3_line(screen_y),
        BgLayer::Bg4 => core.presented_bg4_line(screen_y),
    }
}

fn hdma_targets_bbus(core: &Core, targets: &[u8]) -> bool {
    let hdmaen = core.peek(0x00420C);
    for channel in 0..8 {
        if hdmaen & (1 << channel) == 0 {
            continue;
        }

        let base = 0x004300 + channel * 0x10;
        let dmap = core.peek(base);
        let bbad = core.peek(base + 0x01);
        for offset in dma_transfer_offsets(dmap) {
            let target = bbad.wrapping_add(*offset);
            if targets.contains(&target) {
                return true;
            }
        }
    }
    false
}

fn dma_transfer_offsets(dmap: u8) -> &'static [u8] {
    match dmap & 0x07 {
        0 => &[0],
        1 => &[0, 1],
        2 | 6 => &[0, 0],
        3 | 7 => &[0, 0, 1, 1],
        4 => &[0, 1, 2, 3],
        5 => &[0, 1, 0, 1],
        _ => &[0],
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error(
        "unsupported BG mode {mode}; SNES renderer expects a normal SNES BG mode in the range 0-7"
    )]
    UnsupportedBgMode { mode: u8 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedScreen {
    pub rgba: Vec<u8>,
    pub width: usize,
    pub height: usize,
}

pub fn render_screen(core: &Core) -> Result<RenderedScreen, RenderError> {
    let tm = core.peek(0x00212C);
    let ts = core.peek(0x00212D);
    let use_presented_tm: bool = use_presented_main_screen(core);
    let use_presented_inidisp = hdma_targets_bbus(core, &[0x00]);
    let cgram_hdma_active = hdma_targets_bbus(core, &[0x21]);

    let bgmode = core.peek(0x002105);
    let screen_mode = bgmode & 0x07;
    let high_res_mode = screen_mode == 5 || screen_mode == 6;
    let interlace_enabled = core.interlace_enabled();
    let obj_interlace = core.obj_interlace_enabled();
    let pseudo_hires = core.pseudo_hires_enabled();
    let color_math_supported = screen_mode <= 4 || pseudo_hires;

    let render_width = if high_res_mode || pseudo_hires {
        MODE5_6_WIDTH
    } else {
        SCREEN_WIDTH
    };
    let render_height = if interlace_enabled {
        INTERLACE_HEIGHT
    } else {
        SCREEN_HEIGHT
    };
    let pixel_count = render_width * render_height;

    if tm == 0 && !use_presented_tm {
        return Ok(RenderedScreen {
            rgba: render_presented_backdrop(
                core,
                render_width,
                render_height,
                use_presented_inidisp,
                cgram_hdma_active,
            ),
            width: render_width,
            height: render_height,
        });
    }

    let inidisp = core.peek(0x002100);
    let brightness = inidisp & 0x0F;
    if brightness == 0 && !use_presented_inidisp {
        return Ok(RenderedScreen {
            rgba: opaque_black_screen(render_width, render_height),
            width: render_width,
            height: render_height,
        });
    }

    let render_brightness = if brightness == 0 { 15 } else { brightness };

    // --- Render backdrop to RGBA ---
    let mut rgba = render_presented_backdrop(
        core,
        render_width,
        render_height,
        use_presented_inidisp,
        cgram_hdma_active,
    );

    // --- Main screen: render BG layers to raw 15-bit buffer ---
    let mut main_raw = vec![TRANSPARENT; pixel_count];

    render_bg1(
        core,
        BgLayer::Bg4,
        render_brightness,
        tm,
        use_presented_tm,
        interlace_enabled,
        render_width,
        render_height,
        &mut rgba,
        &mut main_raw,
        0,
    )?;
    render_bg1(
        core,
        BgLayer::Bg3,
        render_brightness,
        tm,
        use_presented_tm,
        interlace_enabled,
        render_width,
        render_height,
        &mut rgba,
        &mut main_raw,
        0,
    )?;
    render_bg1(
        core,
        BgLayer::Bg2,
        render_brightness,
        tm,
        use_presented_tm,
        interlace_enabled,
        render_width,
        render_height,
        &mut rgba,
        &mut main_raw,
        0,
    )?;
    render_bg1(
        core,
        BgLayer::Bg1,
        render_brightness,
        tm,
        use_presented_tm,
        interlace_enabled,
        render_width,
        render_height,
        &mut rgba,
        &mut main_raw,
        if high_res_mode { 1 } else { 0 },
    )?;

    // --- Sub screen: render BG layers for color math and Mode 5/6 interleaving ---
    let mut sub_raw = vec![TRANSPARENT; pixel_count];
    if ts != 0 {
        render_bg1(
            core,
            BgLayer::Bg4,
            render_brightness,
            ts,
            use_presented_tm,
            interlace_enabled,
            render_width,
            render_height,
            &mut rgba,
            &mut sub_raw,
            0,
        )?;
        render_bg1(
            core,
            BgLayer::Bg3,
            render_brightness,
            ts,
            use_presented_tm,
            interlace_enabled,
            render_width,
            render_height,
            &mut rgba,
            &mut sub_raw,
            0,
        )?;
        render_bg1(
            core,
            BgLayer::Bg2,
            render_brightness,
            ts,
            use_presented_tm,
            interlace_enabled,
            render_width,
            render_height,
            &mut rgba,
            &mut sub_raw,
            0,
        )?;
        render_bg1(
            core,
            BgLayer::Bg1,
            render_brightness,
            ts,
            use_presented_tm,
            interlace_enabled,
            render_width,
            render_height,
            &mut rgba,
            &mut sub_raw,
            0,
        )?;

        if color_math_supported {
            let cgwsel = core.peek(0x002130);
            let cgadsub = core.peek(0x002131);
            let fixed_color = core.fixed_color();
            let cgwsel_enable_main = (cgwsel >> 0) & 0x03;
            let cgwsel_disable_main = (cgwsel >> 4) & 0x03;

            let wobjsel = core.peek(0x002125);
            let settings = (wobjsel >> 4) & 0x0F;
            let window1_setting = settings & 0x03;
            let window2_setting = (settings >> 2) & 0x03;
            let in_color_window = window1_setting == 0 && window2_setting == 0;

            let cgadsub_bg1 = cgadsub & 0x01 != 0;
            let cgadsub_bg2 = cgadsub & 0x02 != 0;
            let cgadsub_bg3 = cgadsub & 0x04 != 0;
            let cgadsub_bg4 = cgadsub & 0x08 != 0;
            let cgadsub_obj = cgadsub & 0x10 != 0;
            let cgadsub_backdrop = cgadsub & 0x20 != 0;
            let subtract = cgadsub & 0x80 != 0;
            let half = cgadsub & 0x40 != 0;

            let backdrop_color0 =
                u16::from_le_bytes([core.peek_cgram(0), core.peek_cgram(1)]) & 0x7FFF;

            for i in 0..pixel_count {
                let main_raw_val = main_raw[i];
                let sub_raw_val = sub_raw[i];

                if main_raw_val == TRANSPARENT {
                    continue;
                }

                let layer_participates = cgadsub_bg1
                    || cgadsub_bg2
                    || cgadsub_bg3
                    || cgadsub_bg4
                    || cgadsub_obj
                    || (cgadsub_backdrop && main_raw_val == backdrop_color0);
                if !layer_participates {
                    continue;
                }

                let enable = match cgwsel_enable_main {
                    0 => false,
                    1 => !in_color_window,
                    2 => in_color_window,
                    _ => true,
                };
                if !enable {
                    continue;
                }

                let disable = match cgwsel_disable_main {
                    0 => false,
                    1 => !in_color_window,
                    2 => in_color_window,
                    _ => true,
                };
                if disable {
                    continue;
                }

                let sub_source = if sub_raw_val != TRANSPARENT {
                    sub_raw_val
                } else {
                    fixed_color
                };
                main_raw[i] = apply_color_math(main_raw_val, sub_source, subtract, half);
            }
        }
    }

    // --- Composite BG raw data onto RGBA backdrop ---
    for i in 0..pixel_count {
        let raw = if (high_res_mode || pseudo_hires) && ts != 0 {
            if interlace_enabled {
                let screen_y = i / render_width;
                if screen_y & 1 != 0 { sub_raw[i] } else { main_raw[i] }
            } else {
                let screen_x = i % render_width;
                if screen_x & 1 != 0 { main_raw[i] } else { sub_raw[i] }
            }
        } else {
            main_raw[i]
        };
        if raw != TRANSPARENT {
            let color = snes_color_to_rgba(raw, render_brightness);
            let offset = i * 4;
            rgba[offset..offset + 4].copy_from_slice(&color);
        }
    }

    render_obj(
        core,
        render_brightness,
        tm,
        use_presented_tm,
        interlace_enabled,
        obj_interlace,
        render_width,
        render_height,
        &mut rgba,
    );

    // Apply per-scanline INIDISP forced blanking: scanlines with
    // forced blanking (bit 7) or zero brightness must be black.
    if use_presented_inidisp {
        for screen_y in 0..render_height {
            let presented_y = screen_y / (render_height / SCREEN_HEIGHT).max(1);
            if let Some(backdrop) = core.presented_backdrop_line(presented_y) {
                if backdrop.inidisp & 0x80 != 0 || backdrop.inidisp & 0x0F == 0 {
                    let row_start = screen_y * render_width * 4;
                    for pixel in rgba[row_start..row_start + render_width * 4].chunks_exact_mut(4) {
                        pixel[0] = 0;
                        pixel[1] = 0;
                        pixel[2] = 0;
                        pixel[3] = 0xFF;
                    }
                }
            }
        }
    }

    Ok(RenderedScreen {
        rgba,
        width: render_width,
        height: render_height,
    })
}

#[cfg(test)]
mod tests {
    use super::render_screen;
    use nerust_snes_core::{Core, CpuState};

    const HEADER_OFFSET: usize = 0x7FC0;
    const RESET_VECTOR_OFFSET: usize = 0x7FFC;

    fn build_lorom(reset_vector: u16) -> Vec<u8> {
        let mut rom = vec![0; 0x10000];
        rom[HEADER_OFFSET..HEADER_OFFSET + 21].copy_from_slice(b"TEST SCREEN ROM      ");
        rom[0x7FD5] = 0x30;
        rom[0x7FD7] = 0x08;
        rom[RESET_VECTOR_OFFSET..RESET_VECTOR_OFFSET + 2]
            .copy_from_slice(&reset_vector.to_le_bytes());
        rom
    }

    fn run_until_stopped(core: &mut Core, max_steps: usize) {
        for _ in 0..max_steps {
            core.step().unwrap();
            if core.current_state() == CpuState::Stopped {
                return;
            }
        }

        panic!("core did not stop within {max_steps} steps");
    }

    #[test]
    fn brightness_zero_renders_opaque_black_frame() {
        let core = Core::from_rom_bytes(&build_lorom(0x8000)).unwrap();

        let rendered = render_screen(&core).unwrap();

        assert_eq!(&rendered.rgba[..4], &[0x00, 0x00, 0x00, 0xFF]);
        assert_eq!(
            &rendered.rgba[rendered.rgba.len() - 4..],
            &[0x00, 0x00, 0x00, 0xFF]
        );
    }

    #[test]
    fn mode0_bg1_uses_the_first_cgram_palette_block() {
        let program = [
            0x18, 0xFB, 0xC2, 0x30, 0xE2, 0x20, 0xA9, 0x8F, 0x8D, 0x00, 0x21, 0x9C, 0x05, 0x21,
            0xA9, 0x01, 0x8D, 0x2C, 0x21, 0x9C, 0x07, 0x21, 0xA9, 0x01, 0x8D, 0x0B, 0x21, 0xA9,
            0x80, 0x8D, 0x15, 0x21, 0x9C, 0x16, 0x21, 0xA9, 0x10, 0x8D, 0x17, 0x21, 0xA9, 0xFF,
            0x8D, 0x18, 0x21, 0x8D, 0x19, 0x21, 0x8D, 0x18, 0x21, 0x8D, 0x19, 0x21, 0x8D, 0x18,
            0x21, 0x8D, 0x19, 0x21, 0x8D, 0x18, 0x21, 0x8D, 0x19, 0x21, 0x8D, 0x18, 0x21, 0x8D,
            0x19, 0x21, 0x8D, 0x18, 0x21, 0x8D, 0x19, 0x21, 0x8D, 0x18, 0x21, 0x8D, 0x19, 0x21,
            0x8D, 0x18, 0x21, 0x8D, 0x19, 0x21, 0x9C, 0x21, 0x21, 0xA9, 0x1F, 0x8D, 0x22, 0x21,
            0x9C, 0x22, 0x21, 0xA9, 0x03, 0x8D, 0x21, 0x21, 0xA9, 0xFF, 0x8D, 0x22, 0x21, 0xA9,
            0x7F, 0x8D, 0x22, 0x21, 0xA9, 0x0F, 0x8D, 0x00, 0x21, 0xDB,
        ];
        let mut rom = build_lorom(0x8000);
        rom[..program.len()].copy_from_slice(&program);

        let mut core = Core::from_rom_bytes(&rom).unwrap();
        run_until_stopped(&mut core, 256);

        let rendered = render_screen(&core).unwrap();
        assert_eq!(&rendered.rgba[..4], &[0xFF, 0xFF, 0xFF, 0xFF]);
        assert_eq!(
            &rendered.rgba[rendered.rgba.len() - 4..],
            &[0xFF, 0xFF, 0xFF, 0xFF]
        );
    }

    #[test]
    fn backdrop_color_math_renders_under_enabled_main_screen_layers() {
        let program = [
            0x18, 0xFB, 0xC2, 0x30, 0xE2, 0x20, 0xA9, 0x8F, 0x8D, 0x00, 0x21, 0x9C, 0x21, 0x21,
            0xA9, 0xFF, 0x8D, 0x22, 0x21, 0xA9, 0x7F, 0x8D, 0x22, 0x21, 0x9C, 0x26, 0x21, 0xA9,
            0xFF, 0x8D, 0x27, 0x21, 0xA9, 0x20, 0x8D, 0x25, 0x21, 0x9C, 0x2B, 0x21, 0xA9, 0x90,
            0x8D, 0x30, 0x21, 0xA9, 0x20, 0x8D, 0x31, 0x21, 0xA9, 0x3F, 0x8D, 0x32, 0x21, 0xA9,
            0x01, 0x8D, 0x2C, 0x21, 0xA9, 0x0F, 0x8D, 0x00, 0x21, 0xDB,
        ];
        let mut rom = build_lorom(0x8000);
        rom[..program.len()].copy_from_slice(&program);

        let mut core = Core::from_rom_bytes(&rom).unwrap();
        run_until_stopped(&mut core, 256);

        let rendered = render_screen(&core).unwrap();
        assert_eq!(&rendered.rgba[..4], &[0xFF, 0x00, 0x00, 0xFF]);
    }

    #[test]
    fn mode7_bg1_uses_tilemap_low_bytes_and_tile_pixels_high_bytes() {
        let program = [
            0x18, 0xFB, 0xC2, 0x30, 0xE2, 0x20, 0xA9, 0x8F, 0x8D, 0x00, 0x21, 0xA9, 0x07, 0x8D,
            0x05, 0x21, 0xA9, 0x01, 0x8D, 0x2C, 0x21, 0x9C, 0x1A, 0x21, 0x9C, 0x1B, 0x21, 0xA9,
            0x01, 0x8D, 0x1B, 0x21, 0x9C, 0x1E, 0x21, 0xA9, 0x01, 0x8D, 0x1E, 0x21, 0x9C, 0x15,
            0x21, 0x9C, 0x16, 0x21, 0x9C, 0x17, 0x21, 0xA9, 0x02, 0x8D, 0x18, 0x21, 0xA9, 0x88,
            0x8D, 0x16, 0x21, 0x9C, 0x17, 0x21, 0xA9, 0x05, 0x8D, 0x19, 0x21, 0xA9, 0x05, 0x8D,
            0x21, 0x21, 0xA9, 0x1F, 0x8D, 0x22, 0x21, 0x9C, 0x22, 0x21, 0xA9, 0x0F, 0x8D, 0x00,
            0x21, 0xDB,
        ];
        let mut rom = build_lorom(0x8000);
        rom[..program.len()].copy_from_slice(&program);

        let mut core = Core::from_rom_bytes(&rom).unwrap();
        run_until_stopped(&mut core, 256);

        let rendered = render_screen(&core).unwrap();
        assert_eq!(&rendered.rgba[..4], &[0xFF, 0x00, 0x00, 0xFF]);
    }
}

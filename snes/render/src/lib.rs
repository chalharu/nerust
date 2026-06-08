mod backdrop;
mod bg1;
mod color;
mod mode7;
mod obj;
mod tile;

use backdrop::{render_current_backdrop, render_presented_backdrop};
use bg1::render_bg1;
use color::opaque_black_screen;
use nerust_snes_core::Core;
use obj::render_obj;

pub const SCREEN_WIDTH: usize = 256;
pub const SCREEN_HEIGHT: usize = 224;
pub(crate) const VISIBLE_BG_Y_OFFSET: usize = 1;

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
}

pub fn render_screen(core: &Core) -> Result<RenderedScreen, RenderError> {
    let tm = core.peek(0x00212C);
    let use_presented_tm: bool = use_presented_main_screen(core);
    let use_presented_inidisp = hdma_targets_bbus(core, &[0x00]);

    if tm == 0 && !use_presented_tm {
        return Ok(RenderedScreen {
            rgba: render_presented_backdrop(core),
        });
    }

    let inidisp = core.peek(0x002100);
    let brightness = inidisp & 0x0F;
    if brightness == 0 && !use_presented_inidisp {
        return Ok(RenderedScreen {
            rgba: opaque_black_screen(),
        });
    }

    let render_brightness = if brightness == 0 { 15 } else { brightness };
    let mut rgba = if use_presented_inidisp {
        render_presented_backdrop(core)
    } else {
        render_current_backdrop(core)
    };

    render_bg1(
        core,
        BgLayer::Bg1,
        render_brightness,
        tm,
        use_presented_tm,
        &mut rgba,
    )?;
    render_bg1(
        core,
        BgLayer::Bg2,
        render_brightness,
        tm,
        use_presented_tm,
        &mut rgba,
    )?;
    render_bg1(
        core,
        BgLayer::Bg3,
        render_brightness,
        tm,
        use_presented_tm,
        &mut rgba,
    )?;
    render_bg1(
        core,
        BgLayer::Bg4,
        render_brightness,
        tm,
        use_presented_tm,
        &mut rgba,
    )?;
    render_obj(core, render_brightness, tm, use_presented_tm, &mut rgba);

    // When HDMA changes INIDISP per-scanline, apply per-line force-blank/brightness=0
    if use_presented_inidisp {
        for screen_y in 0..SCREEN_HEIGHT {
            if let Some(backdrop) = core.presented_backdrop_line(screen_y)
                && (backdrop.inidisp & 0x80 != 0 || backdrop.inidisp & 0x0F == 0)
            {
                let offset = screen_y * SCREEN_WIDTH * 4;
                for pixel in rgba[offset..offset + SCREEN_WIDTH * 4].chunks_exact_mut(4) {
                    pixel[0] = 0;
                    pixel[1] = 0;
                    pixel[2] = 0;
                    pixel[3] = 0xFF;
                }
            }
        }
    }

    Ok(RenderedScreen { rgba })
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

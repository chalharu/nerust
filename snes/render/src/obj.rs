use nerust_snes_core::Core;

use super::{
    SCREEN_HEIGHT,
    color::{cgram_color_rgba, put_pixel},
    main_screen_for_line,
    tile::chr_4bpp_pixel,
};

const OBJ_TILE_SIZE: u8 = 8;
const OBJ_SPRITES_PER_SCANLINE_LIMIT: usize = 32;
const OBJ_TILE_SLIVERS_PER_SCANLINE_LIMIT: usize = 34;

pub(super) fn render_obj(
    core: &Core,
    brightness: u8,
    current_tm: u8,
    use_presented_tm: bool,
    interlace_enabled: bool,
    render_width: usize,
    render_height: usize,
    rgba: &mut [u8],
) {
    if !screen_uses_obj(core, current_tm, use_presented_tm, render_height) {
        return;
    }

    let obsel = core.peek(0x002101);
    let (small_size, large_size) = obj_size_pair((obsel >> 5) & 0x07);
    let sprites = collect_obj_sprites(core, small_size, large_size);

    let height_ratio = (render_height / SCREEN_HEIGHT).max(1);

    for screen_y in 0..render_height {
        let presented_y = screen_y / height_ratio;
        if main_screen_for_line(core, presented_y, current_tm, use_presented_tm) & 0x10 == 0 {
            continue;
        }
        let interlace_field = interlace_enabled && (screen_y & 1) == 1;
        let interlace_screen_y = if interlace_field {
            presented_y.wrapping_add(1)
        } else {
            presented_y
        };
        let slivers = obj_slivers_for_scanline(&sprites, interlace_screen_y);
        for sliver in slivers.iter().rev() {
            render_obj_sliver(core, obsel, brightness, rgba, render_width, screen_y, *sliver);
        }
    }
}

fn screen_uses_obj(core: &Core, current_tm: u8, use_presented_tm: bool, render_height: usize) -> bool {
    if !use_presented_tm {
        return current_tm & 0x10 != 0;
    }

    let height_ratio = (render_height / SCREEN_HEIGHT).max(1);
    (0..render_height).step_by(height_ratio).any(|screen_y| {
        main_screen_for_line(core, screen_y / height_ratio, current_tm, use_presented_tm) & 0x10 != 0
    })
}

fn collect_obj_sprites(core: &Core, small_size: ObjSize, large_size: ObjSize) -> Vec<ObjSprite> {
    (0..128)
        .map(|sprite_index| {
            let base = sprite_index * 4;
            let x_low = core.peek_oam(base);
            let y = core.peek_oam(base + 1);
            let tile = core.peek_oam(base + 2);
            let attributes = core.peek_oam(base + 3);
            let extra = core.peek_oam(512 + sprite_index / 4);
            let pair_shift = (sprite_index % 4) * 2;
            let x_high = (extra >> pair_shift) & 0x01 != 0;
            let large = (extra >> (pair_shift + 1)) & 0x01 != 0;
            let size = if large { large_size } else { small_size };

            let x = if x_high {
                i16::from(x_low) - 256
            } else {
                i16::from(x_low)
            };
            let mut y = i16::from(y);
            if y >= SCREEN_HEIGHT as i16 {
                y -= 256;
            }

            ObjSprite {
                x,
                y,
                tile,
                attributes,
                size,
            }
        })
        .collect()
}

fn obj_slivers_for_scanline(sprites: &[ObjSprite], screen_y: usize) -> Vec<ObjSliver> {
    let mut selected_sprites = 0;
    let mut slivers = Vec::new();

    for &sprite in sprites {
        if !obj_contains_scanline(sprite, screen_y) {
            continue;
        }
        if selected_sprites == OBJ_SPRITES_PER_SCANLINE_LIMIT {
            break;
        }
        selected_sprites += 1;

        let columns = sprite.size.width / OBJ_TILE_SIZE;
        for tile_column in 0..columns {
            if slivers.len() == OBJ_TILE_SLIVERS_PER_SCANLINE_LIMIT {
                return slivers;
            }
            slivers.push(ObjSliver {
                sprite,
                tile_column,
            });
        }
    }

    slivers
}

fn obj_contains_scanline(sprite: ObjSprite, screen_y: usize) -> bool {
    let screen_y = screen_y as i16;
    let height = i16::from(sprite.size.height);
    screen_y >= sprite.y && screen_y < sprite.y + height
}

fn render_obj_sliver(
    core: &Core,
    obsel: u8,
    brightness: u8,
    rgba: &mut [u8],
    render_width: usize,
    screen_y: usize,
    sliver: ObjSliver,
) {
    let sprite_y = screen_y as i16 - sliver.sprite.y;
    let source_y = if sliver.sprite.attributes & 0x80 != 0 {
        sliver.sprite.size.height - 1 - sprite_y as u8
    } else {
        sprite_y as u8
    };
    let tile_row = usize::from(source_y / OBJ_TILE_SIZE);
    let pixel_y = usize::from(source_y % OBJ_TILE_SIZE);
    let sliver_x_start = sliver.tile_column * OBJ_TILE_SIZE;

    for pixel_in_sliver in 0..OBJ_TILE_SIZE {
        let sprite_x = sliver_x_start + pixel_in_sliver;
        let target_x = sliver.sprite.x + i16::from(sprite_x);
        if !(0..render_width as i16).contains(&target_x) {
            continue;
        }

        let source_x = if sliver.sprite.attributes & 0x40 != 0 {
            sliver.sprite.size.width - 1 - sprite_x
        } else {
            sprite_x
        };
        let tile_column = usize::from(source_x / OBJ_TILE_SIZE);
        let pixel_x = usize::from(source_x % OBJ_TILE_SIZE);
        let tile_number = (usize::from(sliver.sprite.tile)
            | (usize::from(sliver.sprite.attributes & 0x01) << 8))
            + tile_column
            + tile_row * 16;
        let tile_addr = obj_tile_address(obsel, tile_number);
        let color = chr_4bpp_pixel(core, tile_addr, pixel_x, pixel_y);
        if color == 0 {
            continue;
        }

        let palette = usize::from((sliver.sprite.attributes >> 1) & 0x07);
        let color = cgram_color_rgba(core, 128 + palette * 16 + usize::from(color), brightness);
        put_pixel(rgba, render_width, target_x as usize, screen_y, color);
    }
}

fn obj_tile_address(obsel: u8, tile_number: usize) -> usize {
    let base = usize::from(obsel & 0x07) * 0x4000;
    let gap = usize::from((obsel >> 3) & 0x03) * 0x2000;
    base + tile_number * 32 + ((tile_number >> 8) * gap)
}

fn obj_size_pair(size_select: u8) -> (ObjSize, ObjSize) {
    match size_select {
        0 => (ObjSize::new(8, 8), ObjSize::new(16, 16)),
        1 => (ObjSize::new(8, 8), ObjSize::new(32, 32)),
        2 => (ObjSize::new(8, 8), ObjSize::new(64, 64)),
        3 => (ObjSize::new(16, 16), ObjSize::new(32, 32)),
        4 => (ObjSize::new(16, 16), ObjSize::new(64, 64)),
        5 => (ObjSize::new(32, 32), ObjSize::new(64, 64)),
        6 => (ObjSize::new(16, 32), ObjSize::new(32, 64)),
        _ => (ObjSize::new(16, 32), ObjSize::new(32, 32)),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ObjSize {
    width: u8,
    height: u8,
}

impl ObjSize {
    const fn new(width: u8, height: u8) -> Self {
        Self { width, height }
    }
}

#[derive(Debug, Clone, Copy)]
struct ObjSprite {
    x: i16,
    y: i16,
    tile: u8,
    attributes: u8,
    size: ObjSize,
}

#[derive(Debug, Clone, Copy)]
struct ObjSliver {
    sprite: ObjSprite,
    tile_column: u8,
}

#[cfg(test)]
mod tests {
    use super::{ObjSize, ObjSprite, obj_slivers_for_scanline, obj_tile_address};

    #[test]
    fn obj_tile_address_applies_gap_to_secondary_page() {
        assert_eq!(obj_tile_address(0b0000_1000, 0x00FF), 0x1FE0);
        assert_eq!(obj_tile_address(0b0000_1000, 0x0100), 0x4000);
    }

    #[test]
    fn obj_scanline_selection_keeps_only_first_thirty_two_sprites() {
        let sprites = (0..36)
            .map(|index| test_obj(index, 0, 8, 8))
            .collect::<Vec<_>>();

        let slivers = obj_slivers_for_scanline(&sprites, 0);

        assert_eq!(slivers.len(), 32);
        assert_eq!(slivers.first().unwrap().sprite.tile, 0);
        assert_eq!(slivers.last().unwrap().sprite.tile, 31);
    }

    #[test]
    fn obj_scanline_selection_keeps_only_first_thirty_four_tile_slivers() {
        let sprites = (0..12)
            .map(|index| test_obj(index, 0, 32, 32))
            .collect::<Vec<_>>();

        let slivers = obj_slivers_for_scanline(&sprites, 0);

        assert_eq!(slivers.len(), 34);
        assert_eq!(slivers[31].sprite.tile, 7);
        assert_eq!(slivers[31].tile_column, 3);
        assert_eq!(slivers[32].sprite.tile, 8);
        assert_eq!(slivers[32].tile_column, 0);
        assert_eq!(slivers[33].sprite.tile, 8);
        assert_eq!(slivers[33].tile_column, 1);
    }

    #[test]
    fn obj_scanline_selection_counts_offscreen_sprite_tile_slivers() {
        let sprites = (0..9)
            .map(|index| test_obj(index, -256, 32, 32))
            .collect::<Vec<_>>();

        let slivers = obj_slivers_for_scanline(&sprites, 0);

        assert_eq!(slivers.len(), 34);
        assert_eq!(slivers.last().unwrap().sprite.tile, 8);
    }

    fn test_obj(index: usize, x: i16, width: u8, height: u8) -> ObjSprite {
        ObjSprite {
            x,
            y: 0,
            tile: index as u8,
            attributes: 0,
            size: ObjSize::new(width, height),
        }
    }
}

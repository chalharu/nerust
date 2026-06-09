use nerust_snes_core::{Core, PresentedBackdropLine, PresentedColorWindowLine};

use super::color::{opaque_black_screen, put_pixel, snes_color_to_rgba};

const COLOR_WINDOW_SHIFT: u8 = 4;
const WINDOW1_ENABLE: u8 = 0x02;
const WINDOW1_OUTSIDE: u8 = 0x01;
const WINDOW2_ENABLE: u8 = 0x08;
const WINDOW2_OUTSIDE: u8 = 0x04;
const COLOR_WINDOW_LOGIC_SHIFT: u8 = 2;
const COLOR_WINDOW_SELECTOR_MASK: u8 = 0x03;
const CGWSEL_CLIP_SHIFT: u8 = 6;
const CGWSEL_PREVENT_SHIFT: u8 = 4;
const CGADSUB_SUBTRACT: u8 = 0x80;
const CGADSUB_HALF: u8 = 0x40;
const CGADSUB_ENABLE_BACKDROP: u8 = 0x20;

pub(super) fn render_presented_backdrop(core: &Core, width: usize, height: usize) -> Vec<u8> {
    let fallback_backdrop = current_backdrop_line(core);
    let fallback_window = current_color_window_line(core);
    let color_math = BackdropColorMath::from_core(core);
    let mut rgba = opaque_black_screen(width, height);

    for screen_y in 0..height {
        let presented_y = screen_y / (height / 224).max(1);
        let backdrop = core
            .presented_backdrop_line(presented_y)
            .unwrap_or(fallback_backdrop);
        let window = core
            .presented_color_window_line(presented_y)
            .unwrap_or(fallback_window);
        for screen_x in 0..width {
            let line_color = presented_backdrop_pixel_rgba(backdrop, window, screen_x, color_math);
            put_pixel(&mut rgba, width, screen_x, screen_y, line_color);
        }
    }

    rgba
}

pub(super) fn render_current_backdrop(core: &Core, width: usize, height: usize) -> Vec<u8> {
    let backdrop = current_backdrop_line(core);
    let fallback_window = current_color_window_line(core);
    let color_math = BackdropColorMath::from_core(core);
    let mut rgba = opaque_black_screen(width, height);

    for screen_y in 0..height {
        let presented_y = screen_y / (height / 224).max(1);
        let window = core
            .presented_color_window_line(presented_y)
            .unwrap_or(fallback_window);
        for screen_x in 0..width {
            let line_color = presented_backdrop_pixel_rgba(backdrop, window, screen_x, color_math);
            put_pixel(&mut rgba, width, screen_x, screen_y, line_color);
        }
    }

    rgba
}

fn current_backdrop_line(core: &Core) -> PresentedBackdropLine {
    PresentedBackdropLine {
        inidisp: core.peek(0x002100),
        color0: u16::from_le_bytes([core.peek_cgram(0), core.peek_cgram(1)]) & 0x7FFF,
    }
}

fn current_color_window_line(core: &Core) -> PresentedColorWindowLine {
    PresentedColorWindowLine {
        wh0: core.peek(0x002126),
        wh1: core.peek(0x002127),
        wh2: core.peek(0x002128),
        wh3: core.peek(0x002129),
    }
}

fn presented_backdrop_pixel_rgba(
    line: PresentedBackdropLine,
    window: PresentedColorWindowLine,
    screen_x: usize,
    color_math: BackdropColorMath,
) -> [u8; 4] {
    let brightness = line.inidisp & 0x0F;
    if line.inidisp & 0x80 != 0 || brightness == 0 {
        [0x00, 0x00, 0x00, 0xFF]
    } else {
        snes_color_to_rgba(
            color_math.apply_to_backdrop(line.color0, window, screen_x),
            brightness,
        )
    }
}

#[derive(Debug, Clone, Copy)]
struct BackdropColorMath {
    wobjsel: u8,
    wobjlog: u8,
    cgwsel: u8,
    cgadsub: u8,
    fixed_color: u16,
}

impl BackdropColorMath {
    fn from_core(core: &Core) -> Self {
        Self {
            wobjsel: core.peek(0x002125),
            wobjlog: core.peek(0x00212B),
            cgwsel: core.peek(0x002130),
            cgadsub: core.peek(0x002131),
            fixed_color: core.fixed_color(),
        }
    }

    fn apply_to_backdrop(
        self,
        color: u16,
        window: PresentedColorWindowLine,
        screen_x: usize,
    ) -> u16 {
        let in_color_window = self.in_color_window(window, screen_x);
        let clipped = selector_matches((self.cgwsel >> CGWSEL_CLIP_SHIFT) & 0x03, in_color_window);
        let prevented = selector_matches(
            (self.cgwsel >> CGWSEL_PREVENT_SHIFT) & 0x03,
            in_color_window,
        );
        let main_color = if clipped { 0 } else { color };
        if prevented || self.cgadsub & CGADSUB_ENABLE_BACKDROP == 0 {
            return main_color;
        }

        add_subtract_color(
            main_color,
            self.fixed_color,
            self.cgadsub & CGADSUB_SUBTRACT != 0,
            self.cgadsub & CGADSUB_HALF != 0,
        )
    }

    fn in_color_window(self, window: PresentedColorWindowLine, screen_x: usize) -> bool {
        let settings = (self.wobjsel >> COLOR_WINDOW_SHIFT) & 0x0F;
        let win1 = window_state(
            settings & WINDOW1_ENABLE != 0,
            settings & WINDOW1_OUTSIDE != 0,
            window_contains(window.wh0, window.wh1, screen_x),
        );
        let win2 = window_state(
            settings & WINDOW2_ENABLE != 0,
            settings & WINDOW2_OUTSIDE != 0,
            window_contains(window.wh2, window.wh3, screen_x),
        );

        match (win1, win2) {
            (None, None) => true,
            (Some(value), None) | (None, Some(value)) => value,
            (Some(win1), Some(win2)) => {
                match (self.wobjlog >> COLOR_WINDOW_LOGIC_SHIFT) & COLOR_WINDOW_SELECTOR_MASK {
                    0 => win1 || win2,
                    1 => win1 && win2,
                    2 => win1 ^ win2,
                    _ => !(win1 ^ win2),
                }
            }
        }
    }
}

fn window_contains(left: u8, right: u8, screen_x: usize) -> bool {
    left <= right && (usize::from(left)..=usize::from(right)).contains(&screen_x)
}

fn window_state(enabled: bool, outside: bool, contains: bool) -> Option<bool> {
    enabled.then_some(if outside { !contains } else { contains })
}

fn selector_matches(selector: u8, in_window: bool) -> bool {
    match selector {
        0 => false,
        1 => !in_window,
        2 => in_window,
        _ => true,
    }
}

fn add_subtract_color(base: u16, fixed: u16, subtract: bool, half: bool) -> u16 {
    let mut red = combine_channel(base & 0x1F, fixed & 0x1F, subtract);
    let mut green = combine_channel((base >> 5) & 0x1F, (fixed >> 5) & 0x1F, subtract);
    let mut blue = combine_channel((base >> 10) & 0x1F, (fixed >> 10) & 0x1F, subtract);
    if half {
        red /= 2;
        green /= 2;
        blue /= 2;
    }
    red | (green << 5) | (blue << 10)
}

fn combine_channel(base: u16, fixed: u16, subtract: bool) -> u16 {
    if subtract {
        base.saturating_sub(fixed)
    } else {
        (base + fixed).min(0x1F)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BackdropColorMath, add_subtract_color, selector_matches, window_contains, window_state,
    };
    use nerust_snes_core::{PresentedBackdropLine, PresentedColorWindowLine};

    #[test]
    fn color_window_clip_inside_plus_fixed_color_yields_fixed_color() {
        let math = BackdropColorMath {
            wobjsel: 0x20,
            wobjlog: 0x00,
            cgwsel: 0x90,
            cgadsub: 0x20,
            fixed_color: (31 << 10) | 15,
        };
        let window = PresentedColorWindowLine {
            wh0: 10,
            wh1: 20,
            wh2: 0,
            wh3: 0,
        };

        assert_eq!(math.apply_to_backdrop(0x7FFF, window, 9), 0x7FFF);
        assert_eq!(math.apply_to_backdrop(0x7FFF, window, 10), (31 << 10) | 15);
        assert_eq!(math.apply_to_backdrop(0x7FFF, window, 20), (31 << 10) | 15);
        assert_eq!(math.apply_to_backdrop(0x7FFF, window, 21), 0x7FFF);
    }

    #[test]
    fn disabled_window_range_never_contains_pixels() {
        assert!(!window_contains(0xFF, 0x00, 0));
        assert_eq!(window_state(true, false, false), Some(false));
        assert_eq!(window_state(true, true, false), Some(true));
    }

    #[test]
    fn color_math_selectors_match_expected_regions() {
        assert!(!selector_matches(0, true));
        assert!(selector_matches(1, false));
        assert!(selector_matches(2, true));
        assert!(selector_matches(3, false));
    }

    #[test]
    fn color_math_adds_and_subtracts_5bit_channels() {
        assert_eq!(add_subtract_color(0x001F, 0x0001, false, false), 0x001F);
        assert_eq!(add_subtract_color(0x0010, 0x0001, true, false), 0x000F);
        assert_eq!(add_subtract_color(0x0010, 0x0002, false, true), 0x0009);
    }

    #[test]
    fn force_blank_takes_priority_over_color_math() {
        let math = BackdropColorMath {
            wobjsel: 0x20,
            wobjlog: 0x00,
            cgwsel: 0x90,
            cgadsub: 0x20,
            fixed_color: 0x7C1F,
        };
        let line = PresentedBackdropLine {
            inidisp: 0x8F,
            color0: 0x7FFF,
        };
        let window = PresentedColorWindowLine {
            wh0: 0,
            wh1: 255,
            wh2: 0,
            wh3: 0,
        };

        assert_eq!(
            super::presented_backdrop_pixel_rgba(line, window, 0, math),
            [0, 0, 0, 0xFF]
        );
    }
}

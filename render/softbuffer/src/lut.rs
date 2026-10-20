//! Resize LUT: per-axis tap tables plus the bilinear kernel.
//! One rebuild per resize; per-frame work only reads the tables.

use nerust_render_traits::SurfaceSize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResizeKernel {
    // No selection knob yet: Bilinear is hardcoded; kept so the nearest
    // path stays compilable beside the bilinear one.
    #[expect(unused)]
    NearestNeighbor,
    Bilinear,
}

#[derive(Debug)]
pub(crate) struct LutEntry {
    pub(crate) x_lut: Vec<Option<(u16, u16)>>,
    pub(crate) y_lut: Vec<Option<(u16, u16)>>,
    pub(crate) kernel: ResizeKernel,
    pub(crate) rev: u64,
}

/// One bilinear tap: source index plus its 8.8 fixed-point weight.
type TapEntry = Option<(u16, u16)>;

impl LutEntry {
    pub(crate) fn new() -> Self {
        Self {
            x_lut: Vec::new(),
            y_lut: Vec::new(),
            // kernel: ResizeKernel::NearestNeighbor,
            kernel: ResizeKernel::Bilinear,
            rev: 0,
        }
    }

    /// Generation counter: any LUT rebuild invalidates tap verdicts.
    pub(crate) fn bump_rev(&mut self) {
        self.rev = self.rev.wrapping_add(1);
    }

    fn inv_scale(
        source_size: SurfaceSize,
        physical_aspect_ratio: f32,
        destination_size: SurfaceSize,
    ) -> (f32, f32) {
        let window_aspect = destination_size.width as f32 / destination_size.height as f32;
        let source_aspect = source_size.width as f32 / source_size.height as f32;

        if window_aspect > physical_aspect_ratio {
            // Window is wider than content → letterbox (black bars on sides)
            (
                source_size.height as f32 / destination_size.height as f32
                    * (source_aspect / physical_aspect_ratio),
                source_size.height as f32 / destination_size.height as f32,
            )
        } else {
            (
                source_size.width as f32 / destination_size.width as f32,
                source_size.width as f32 / destination_size.width as f32
                    * (physical_aspect_ratio / source_aspect),
            )
        }
    }

    fn lut_reserve(lut: &mut Vec<Option<(u16, u16)>>, len: usize) {
        lut.clear();
        lut.reserve_exact(len);
    }

    pub(crate) fn resize_lut_nearest_neighbor(
        &mut self,
        dst_w: usize,
        dst_h: usize,
        src_w: usize,
        src_h: usize,
        scale: (f32, f32),
    ) {
        self.bump_rev();
        Self::lut_reserve(&mut self.x_lut, dst_w);
        Self::lut_reserve(&mut self.y_lut, dst_h);

        for x in 0..dst_w {
            // Center the source pixel coordinates based on the destination size and scaling factors
            let src_x = ((x as isize - (dst_w >> 1) as isize) as f32 * scale.0 + 0.5) as isize
                + (src_w >> 1) as isize;
            if src_x < 0 || src_x >= src_w as isize {
                self.x_lut.push(None);
            } else {
                self.x_lut.push(Some((src_x as u16, 256)));
            }
        }
        for y in 0..dst_h {
            // Center the source pixel coordinates based on the destination size and scaling factors
            let src_y = ((y as isize - (dst_h >> 1) as isize) as f32 * scale.1 + 0.5) as isize
                + (src_h >> 1) as isize;
            if src_y < 0 || src_y >= src_h as isize {
                self.y_lut.push(None);
            } else {
                self.y_lut.push(Some((src_y as u16, 256)));
            }
        }
    }

    /// Centered source coordinate for one destination pixel.
    fn centered_src(dst: usize, dst_len: usize, factor: f32, src_len: usize) -> f32 {
        ((dst as isize - (dst_len >> 1) as isize) as f32 * factor + 0.5) + (src_len >> 1) as f32
    }

    /// Bilinear tap weights: `(floor, ceil)` entries, `None` when outside.
    fn tap_pair(src: f32, src_len: usize) -> (TapEntry, TapEntry) {
        let floor = src.floor() as isize;
        let ceil = src.ceil() as isize;
        let (mut w_floor, mut w_ceil) = if ceil == floor {
            (256, 0)
        } else {
            (
                ((ceil as f32 - src) * 256.0 + 0.5) as u16,
                ((src - floor as f32) * 256.0 + 0.5) as u16,
            )
        };
        if floor < 0 || floor >= src_len as isize {
            w_floor = 0;
            if w_ceil != 0 {
                w_ceil = 256;
            }
        }
        if ceil < 0 || ceil >= src_len as isize {
            if w_floor != 0 {
                w_floor = 256;
            }
            w_ceil = 0;
        }
        let floor_entry = (w_floor != 0).then_some((floor as u16, w_floor));
        let ceil_entry = (w_ceil != 0).then_some((ceil as u16, w_ceil));
        (floor_entry, ceil_entry)
    }

    pub(crate) fn resize_lut_bilinear(
        &mut self,
        dst_w: usize,
        dst_h: usize,
        src_w: usize,
        src_h: usize,
        scale: (f32, f32),
    ) {
        self.bump_rev();
        Self::lut_reserve(&mut self.x_lut, dst_w * 2);
        Self::lut_reserve(&mut self.y_lut, dst_h * 2);
        for x in 0..dst_w {
            let (floor, ceil) = Self::tap_pair(Self::centered_src(x, dst_w, scale.0, src_w), src_w);
            self.x_lut.push(floor);
            self.x_lut.push(ceil);
        }
        for y in 0..dst_h {
            let (floor, ceil) = Self::tap_pair(Self::centered_src(y, dst_h, scale.1, src_h), src_h);
            self.y_lut.push(floor);
            self.y_lut.push(ceil);
        }
    }

    pub(crate) fn resize_lut(
        &mut self,
        source_size: SurfaceSize,
        physical_aspect_ratio: f32,
        destination_size: SurfaceSize,
    ) {
        self.bump_rev();
        let dst_w = destination_size.width as usize;
        let dst_h = destination_size.height as usize;
        let src_w = source_size.width as usize;
        let src_h = source_size.height as usize;

        let scale = Self::inv_scale(source_size, physical_aspect_ratio, destination_size);

        match self.kernel {
            ResizeKernel::NearestNeighbor => {
                self.resize_lut_nearest_neighbor(dst_w, dst_h, src_w, src_h, scale);
            }
            ResizeKernel::Bilinear => {
                self.resize_lut_bilinear(dst_w, dst_h, src_w, src_h, scale);
            }
        }
    }

    pub(crate) fn lut_pixel_size(&self) -> usize {
        match self.kernel {
            ResizeKernel::NearestNeighbor => 1,
            ResizeKernel::Bilinear => 2,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{LutEntry, ResizeKernel};
    use nerust_render_traits::SurfaceSize;

    #[test]
    fn nearest_and_bilinear_cover_every_dst_pixel() {
        let mut lut = LutEntry::new();
        lut.resize_lut_nearest_neighbor(4, 4, 4, 4, (1.0, 1.0));
        assert_eq!(lut.x_lut.len(), 4);
        lut.resize_lut_bilinear(4, 4, 4, 4, (1.0, 1.0));
        assert_eq!(lut.x_lut.len(), 8);
        assert_eq!(lut.lut_pixel_size(), 2);
        assert_eq!(lut.y_lut.len(), 8);
    }

    #[test]
    fn outside_taps_emit_none_not_wrapped_indices() {
        let mut lut = LutEntry::new();
        lut.resize_lut_nearest_neighbor(8, 1, 2, 1, (4.0, 1.0));
        assert!(lut.x_lut.iter().any(|t| t.is_none()));
        assert!(lut.x_lut.iter().flatten().all(|&(i, _)| i < 2));
    }

    #[test]
    fn centered_src_pins_middle_pixel() {
        assert_eq!(LutEntry::centered_src(1, 2, 1.0, 2), 1.5);
    }

    #[test]
    fn tap_pair_splits_unit_weight() {
        let (floor, ceil) = LutEntry::tap_pair(1.5, 4);
        let (f, wf) = floor.expect("floor tap");
        let (c, wc) = ceil.expect("ceil tap");
        assert_eq!((f, c), (1, 2));
        assert_eq!(wf as u32 + wc as u32, 256);
        let (floor, ceil) = LutEntry::tap_pair(1.0, 4);
        assert!(floor.is_some() && ceil.is_none());
    }

    #[test]
    fn inv_scale_letterboxes_wide_windows() {
        let (sx, sy) = LutEntry::inv_scale(
            SurfaceSize {
                width: 256,
                height: 240,
            },
            4.0 / 3.0,
            SurfaceSize {
                width: 1920,
                height: 1080,
            },
        );
        assert!(sx > 0.0 && sy > 0.0);
        let narrow = LutEntry::inv_scale(
            SurfaceSize {
                width: 256,
                height: 240,
            },
            4.0 / 3.0,
            SurfaceSize {
                width: 600,
                height: 1080,
            },
        );
        assert!(narrow.0 > 0.0 && narrow.1 > 0.0);
    }

    #[test]
    fn resize_lut_dispatches_on_kernel() {
        let mut lut = LutEntry::new();
        lut.kernel = ResizeKernel::Bilinear;
        lut.resize_lut(
            SurfaceSize {
                width: 2,
                height: 2,
            },
            1.0,
            SurfaceSize {
                width: 4,
                height: 4,
            },
        );
        assert_eq!(lut.x_lut.len(), 8);
    }
}

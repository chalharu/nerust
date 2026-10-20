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
            // Center the source pixel coordinates based on the destination size and scaling factors
            let src_x =
                ((x as isize - (dst_w >> 1) as isize) as f32 * scale.0 + 0.5) + (src_w >> 1) as f32;
            let src_x_floor = src_x.floor() as isize;
            let src_x_ceil = src_x.ceil() as isize;
            let (mut weight_floor, mut weight_ceil) = if src_x_ceil == src_x_floor {
                (256, 0) // 同一のピクセルに対しては、floor側に全ての重みを割り当てる
            } else {
                (
                    ((src_x_ceil as f32 - src_x) * 256.0 + 0.5) as u16,
                    ((src_x - src_x_floor as f32) * 256.0 + 0.5) as u16,
                )
            };

            if src_x_floor < 0 || src_x_floor >= src_w as isize {
                weight_floor = 0;
                if weight_ceil != 0 {
                    weight_ceil = 256;
                }
            }
            if src_x_ceil < 0 || src_x_ceil >= src_w as isize {
                if weight_floor != 0 {
                    weight_floor = 256;
                }
                weight_ceil = 0;
            }

            if weight_floor == 0 {
                self.x_lut.push(None);
            } else {
                self.x_lut.push(Some((src_x_floor as u16, weight_floor)));
            }
            if weight_ceil == 0 {
                self.x_lut.push(None);
            } else {
                self.x_lut.push(Some((src_x_ceil as u16, weight_ceil)));
            }
        }
        for y in 0..dst_h {
            // Center the source pixel coordinates based on the destination size and scaling factors
            let src_y =
                ((y as isize - (dst_h >> 1) as isize) as f32 * scale.1 + 0.5) + (src_h >> 1) as f32;
            let src_y_floor = src_y.floor() as isize;
            let src_y_ceil = src_y.ceil() as isize;
            let (mut weight_floor, mut weight_ceil) = if src_y_ceil == src_y_floor {
                (256, 0) // 同一のピクセルに対しては、floor側に全ての重みを割り当てる
            } else {
                (
                    ((src_y_ceil as f32 - src_y) * 256.0 + 0.5) as u16,
                    ((src_y - src_y_floor as f32) * 256.0 + 0.5) as u16,
                )
            };

            if src_y_floor < 0 || src_y_floor >= src_h as isize {
                weight_floor = 0;
                if weight_ceil != 0 {
                    weight_ceil = 256;
                }
            }
            if src_y_ceil < 0 || src_y_ceil >= src_h as isize {
                if weight_floor != 0 {
                    weight_floor = 256;
                }
                weight_ceil = 0;
            }

            if weight_floor == 0 {
                self.y_lut.push(None);
            } else {
                self.y_lut.push(Some((src_y_floor as u16, weight_floor)));
            }
            if weight_ceil == 0 {
                self.y_lut.push(None);
            } else {
                self.y_lut.push(Some((src_y_ceil as u16, weight_ceil)));
            }
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

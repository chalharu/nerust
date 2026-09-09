use super::fixed_point::Fixed8_8;
use super::lut::{cos_fixed, sin_fixed};

/// アフィン行列パラメータ P_A, P_B, P_C, P_D (8.8 固定小数点)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AffineMatrix {
    pub pa: Fixed8_8,
    pub pb: Fixed8_8,
    pub pc: Fixed8_8,
    pub pd: Fixed8_8,
}

/// BgAffineSet ソース
#[derive(Debug, Clone, Copy)]
pub struct BgAffineSrc {
    pub cx: i32, // 20.8
    pub cy: i32,
    pub disp_cx: i16,
    pub disp_cy: i16,
    pub sx: Fixed8_8,
    pub sy: Fixed8_8,
    pub alpha: u16, // 8.8
}

/// BgAffineSet デスティネーション
#[derive(Debug, Clone, Copy)]
pub struct BgAffineDst {
    pub pa: Fixed8_8,
    pub pb: Fixed8_8,
    pub pc: Fixed8_8,
    pub pd: Fixed8_8,
    pub start_x: i32, // 20.8
    pub start_y: i32,
}

pub fn bg_affine_set(src: &BgAffineSrc, dst: &mut BgAffineDst) {
    // mGBA: theta = (alpha>>8)/128 * PI,  sin/cos via table truncated to 8bit
    let sin = sin_fixed(src.alpha) as i32;
    let cos = cos_fixed(src.alpha) as i32;
    let sx = src.sx.to_raw() as i32;
    let sy = src.sy.to_raw() as i32;

    dst.pa = Fixed8_8::from_raw(((cos * sx) >> 8) as i16);
    dst.pb = Fixed8_8::from_raw(((-sin * sx) >> 8) as i16);
    dst.pc = Fixed8_8::from_raw(((sin * sy) >> 8) as i16);
    dst.pd = Fixed8_8::from_raw(((cos * sy) >> 8) as i16);

    // mGBA: rx = ox - (a*cx + b*cy)
    dst.start_x = src.cx
        - (src.disp_cx as i32 * dst.pa.to_raw() as i32
            + src.disp_cy as i32 * dst.pb.to_raw() as i32);
    dst.start_y = src.cy
        - (src.disp_cx as i32 * dst.pc.to_raw() as i32
            + src.disp_cy as i32 * dst.pd.to_raw() as i32);
}

/// ObjAffineSet ソース/dest
#[derive(Debug, Clone, Copy)]
pub struct ObjAffineSrc {
    pub sx: Fixed8_8,
    pub sy: Fixed8_8,
    pub alpha: u16,
}

#[derive(Debug, Clone, Copy)]
pub struct ObjAffineDst {
    pub pa: Fixed8_8,
    pub pb: Fixed8_8,
    pub pc: Fixed8_8,
    pub pd: Fixed8_8,
}

pub fn obj_affine_set(src: &ObjAffineSrc, dst: &mut ObjAffineDst) {
    let sin = sin_fixed(src.alpha) as i32;
    let cos = cos_fixed(src.alpha) as i32;
    let sx = src.sx.to_raw() as i32;
    let sy = src.sy.to_raw() as i32;
    dst.pa = Fixed8_8::from_raw(((cos * sx) >> 8) as i16);
    dst.pb = Fixed8_8::from_raw(((-sin * sx) >> 8) as i16);
    dst.pc = Fixed8_8::from_raw(((sin * sy) >> 8) as i16);
    dst.pd = Fixed8_8::from_raw(((cos * sy) >> 8) as i16);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bg_affine_set_demo_params() {
        // PeterLemon BGRotZoomMode2 defaults: 1024px map centered at
        // (512,512) shown at screen (120,80), scale 1.0, no rotation.
        // rx = ox - (a*cx + b*cy) per mGBA/GBATEK BgAffineSet.
        let s = BgAffineSrc {
            cx: 0x20000,
            cy: 0x20000,
            disp_cx: 0x78,
            disp_cy: 0x50,
            sx: Fixed8_8::from_raw(0x100),
            sy: Fixed8_8::from_raw(0x100),
            alpha: 0,
        };
        let mut d = BgAffineDst {
            pa: Fixed8_8::from_raw(0),
            pb: Fixed8_8::from_raw(0),
            pc: Fixed8_8::from_raw(0),
            pd: Fixed8_8::from_raw(0),
            start_x: 0,
            start_y: 0,
        };
        bg_affine_set(&s, &mut d);
        assert_eq!(d.pa.to_raw(), 0x100);
        assert_eq!(d.pb.to_raw(), 0);
        assert_eq!(d.pc.to_raw(), 0);
        assert_eq!(d.pd.to_raw(), 0x100);
        assert_eq!(d.start_x, 0x18800); // 512 - 120 = 392.0
        assert_eq!(d.start_y, 0x1B000); // 512 - 80 = 432.0
    }

    #[test]
    fn affine_identity() {
        let src = BgAffineSrc {
            cx: 0,
            cy: 0,
            disp_cx: 0,
            disp_cy: 0,
            sx: Fixed8_8::from_int(1),
            sy: Fixed8_8::from_int(1),
            alpha: 0,
        };
        let mut dst = BgAffineDst {
            pa: Fixed8_8::from_raw(0),
            pb: Fixed8_8::from_raw(0),
            pc: Fixed8_8::from_raw(0),
            pd: Fixed8_8::from_raw(0),
            start_x: 0,
            start_y: 0,
        };
        bg_affine_set(&src, &mut dst);
        assert_eq!(dst.pa.to_raw(), 256); // cos 0 *1 =1.0 -> 256
        assert_eq!(dst.pb.to_raw(), 0);
    }
}

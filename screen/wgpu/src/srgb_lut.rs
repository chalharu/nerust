const fn fifth_root(value: f64) -> f64 {
    if value == 0.0 {
        return 0.0;
    }

    let mut root = 1.0;
    let mut iteration = 0;
    while iteration < 20 {
        let root_sq = root * root;
        let root_fourth = root_sq * root_sq;
        root = (4.0 * root + value / root_fourth) / 5.0;
        iteration += 1;
    }
    root
}

const fn srgb_to_linear_value(index: usize) -> f32 {
    let srgb = index as f64 / 255.0;
    let linear = if srgb <= 0.04045 {
        srgb / 12.92
    } else {
        let base = (srgb + 0.055) / 1.055;
        let base_sq = base * base;
        base_sq * fifth_root(base_sq)
    };
    linear as f32
}

const fn srgb_to_linear_lut() -> [f32; 256] {
    let mut lut = [0.0; 256];
    let mut index = 0;
    while index < lut.len() {
        lut[index] = srgb_to_linear_value(index);
        index += 1;
    }
    lut
}

pub(crate) const SRGB_TO_LINEAR_LUT: [f32; 256] = srgb_to_linear_lut();

const SRGB_TO_LINEAR_LUT_BYTE_LEN: usize = SRGB_TO_LINEAR_LUT.len() * std::mem::size_of::<f32>();

const fn encode_lut_bytes(lut: [f32; 256]) -> [u8; SRGB_TO_LINEAR_LUT_BYTE_LEN] {
    let mut bytes = [0; SRGB_TO_LINEAR_LUT_BYTE_LEN];
    let mut index = 0;
    while index < lut.len() {
        let encoded = lut[index].to_le_bytes();
        let byte_index = index * std::mem::size_of::<f32>();
        bytes[byte_index] = encoded[0];
        bytes[byte_index + 1] = encoded[1];
        bytes[byte_index + 2] = encoded[2];
        bytes[byte_index + 3] = encoded[3];
        index += 1;
    }
    bytes
}

pub(crate) const SRGB_TO_LINEAR_LUT_BYTES: [u8; SRGB_TO_LINEAR_LUT_BYTE_LEN] =
    encode_lut_bytes(SRGB_TO_LINEAR_LUT);

#[cfg(test)]
mod tests {
    use super::{SRGB_TO_LINEAR_LUT, SRGB_TO_LINEAR_LUT_BYTES};

    #[test]
    fn lut_bytes_match_source_values() {
        assert_eq!(
            SRGB_TO_LINEAR_LUT_BYTES.len(),
            SRGB_TO_LINEAR_LUT.len() * std::mem::size_of::<f32>()
        );

        for (index, chunk) in SRGB_TO_LINEAR_LUT_BYTES.chunks_exact(4).enumerate() {
            assert_eq!(chunk, &SRGB_TO_LINEAR_LUT[index].to_le_bytes());
        }
    }

    #[test]
    fn lut_matches_reference_formula() {
        for (index, value) in SRGB_TO_LINEAR_LUT.iter().enumerate() {
            let srgb = index as f64 / 255.0;
            let expected = if srgb <= 0.04045 {
                (srgb / 12.92) as f32
            } else {
                (((srgb + 0.055) / 1.055).powf(2.4)) as f32
            };
            assert_eq!(value.to_bits(), expected.to_bits(), "index {index}");
        }
    }
}

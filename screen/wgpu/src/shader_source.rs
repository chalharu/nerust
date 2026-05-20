pub(crate) const NTSC_SAMPLE_COUNT: usize = 7;
pub(crate) const NTSC_TAP_COUNT: usize = 6;
pub(crate) const SRGB_LUT_SIZE: usize = 256;

pub(crate) fn ntsc_row_offsets() -> [[i32; NTSC_TAP_COUNT]; NTSC_SAMPLE_COUNT] {
    std::array::from_fn(|sample| {
        let sample = sample as i32;
        [
            sample,
            (sample + 12) % 7 + 14,
            (sample + 10) % 7 + 28,
            (sample + 7) % 14,
            (sample + 5) % 7 + 21,
            (sample + 3) % 7 + 35,
        ]
    })
}

pub(crate) fn srgb_to_linear_lut() -> [f64; SRGB_LUT_SIZE] {
    std::array::from_fn(|index| {
        let srgb = index as f64 / 255.0;
        if srgb <= 0.04045 {
            srgb / 12.92
        } else {
            ((srgb + 0.055) / 1.055).powf(2.4)
        }
    })
}

pub(crate) fn srgb_to_linear_lut_bytes() -> Vec<u8> {
    srgb_to_linear_lut()
        .into_iter()
        .flat_map(|value| (value as f32).to_le_bytes())
        .collect()
}

pub(crate) fn render_ntsc_row_offsets_wgsl() -> String {
    let rows = ntsc_row_offsets();
    let mut output = String::from("array<array<i32, 6>, 7>(\n");
    for (row_index, row) in rows.iter().enumerate() {
        output.push_str("    array<i32, 6>(");
        for (offset_index, offset) in row.iter().enumerate() {
            if offset_index > 0 {
                output.push_str(", ");
            }
            output.push_str(&offset.to_string());
        }
        output.push(')');
        if row_index + 1 != rows.len() {
            output.push(',');
        }
        output.push('\n');
    }
    output.push(')');
    output
}

pub(crate) fn render_shader_wgsl(template: &str) -> String {
    template.replace("__NTSC_ROW_OFFSETS__", &render_ntsc_row_offsets_wgsl())
}

#[cfg(test)]
mod tests {
    use super::{ntsc_row_offsets, render_shader_wgsl, srgb_to_linear_lut_bytes};

    fn srgb_formula(index: usize) -> f64 {
        let srgb = index as f64 / 255.0;
        if srgb <= 0.04045 {
            srgb / 12.92
        } else {
            ((srgb + 0.055) / 1.055).powf(2.4)
        }
    }

    #[test]
    fn ntsc_offsets_match_expected_rows() {
        assert_eq!(
            ntsc_row_offsets(),
            [
                [0, 19, 31, 7, 26, 38],
                [1, 20, 32, 8, 27, 39],
                [2, 14, 33, 9, 21, 40],
                [3, 15, 34, 10, 22, 41],
                [4, 16, 28, 11, 23, 35],
                [5, 17, 29, 12, 24, 36],
                [6, 18, 30, 13, 25, 37],
            ]
        );
    }

    #[test]
    fn srgb_lut_bytes_match_all_entries() {
        let bytes = srgb_to_linear_lut_bytes();
        assert_eq!(bytes.len(), 256 * std::mem::size_of::<f32>());

        for (index, chunk) in bytes.chunks_exact(4).enumerate() {
            let value = f32::from_le_bytes(chunk.try_into().expect("chunk size must be 4"));
            let expected = srgb_formula(index) as f32;
            assert_eq!(
                value.to_bits(),
                expected.to_bits(),
                "unexpected LUT value at index {index}: {value}"
            );
        }
    }

    #[test]
    fn shader_render_uses_real_template_and_replaces_placeholders() {
        let template = include_str!("shader.wgsl.in");
        assert_eq!(template.matches("__NTSC_ROW_OFFSETS__").count(), 1);

        let rendered = render_shader_wgsl(template);
        assert!(!rendered.contains("__NTSC_ROW_OFFSETS__"));
        assert!(rendered.contains(
            "const NTSC_ROW_OFFSETS: array<array<i32, 6>, 7> = array<array<i32, 6>, 7>("
        ));
        assert!(rendered.contains("@group(0) @binding(4)"));
    }
}

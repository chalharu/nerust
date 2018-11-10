// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Image parameters, ranging from -1.0 to 1.0. Actual internal values shown
// in parenthesis and should remain fairly stable in future versions.
pub struct NesNtscSetup {
    // Basic parameters
    pub hue: f64,        // -1 = -180 degrees     +1 = +180 degrees
    pub saturation: f64, // -1 = grayscale (0.0)  +1 = oversaturated colors (2.0)
    pub contrast: f64,   // -1 = dark (0.5)       +1 = light (1.5)
    pub brightness: f64, // -1 = dark (0.5)       +1 = light (1.5)
    pub sharpness: f64,  // edge contrast enhancement/blurring

    // Advanced parameters
    pub gamma: f64,      // -1 = dark (1.5)       +1 = light (0.5)
    pub resolution: f64, // image resolution
    pub artifacts: f64,  // artifacts caused by color changes
    pub fringing: f64,   // color artifacts caused by brightness changes
    pub bleed: f64,      // color bleed (color resolution reduction)
    pub merge_fields: bool, // if true, merges even and odd fields together to reduce flicker
    // decoder_matrix: &[f32; 6], // optional RGB decoder matrix, 6 elements

    // palette_out: &mut [u8], // optional RGB palette out, 3 bytes per color

    // // You can replace the standard NES color generation with an RGB palette. The
    // // first replaces all color generation, while the second replaces only the core
    // // 64-color generation and does standard color emphasis calculations on it.
    // palette: &[u8; 512 * 3], // optional 512-entry RGB palette in, 3 bytes per color
    // base_palette: &[u8; 64 * 3], // optional 64-entry RGB palette in, 3 bytes per color
}

impl NesNtscSetup {
    // pub fn rgb() -> Self {
    //     Self {
    //         hue: 0.0,
    //         saturation: 0.0,
    //         contrast: 0.0,
    //         brightness: 0.0,
    //         sharpness: 0.2,
    //         gamma: 0.0,
    //         resolution: 0.7,
    //         artifacts: -1.0,
    //         fringing: -1.0,
    //         bleed: -1.0,
    //         merge_fields: true,
    //     }
    // }
    // pub fn monochrome() -> Self {
    //     Self {
    //         hue: 0.0,
    //         saturation: -1.0,
    //         contrast: 0.0,
    //         brightness: 0.0,
    //         sharpness: 0.2,
    //         gamma: 0.0,
    //         resolution: 0.2,
    //         artifacts: -0.2,
    //         fringing: -0.2,
    //         bleed: -1.0,
    //         merge_fields: true,
    //     }
    // }
    pub fn composite() -> Self {
        Self {
            hue: 0.0,
            saturation: 0.0,
            contrast: 0.0,
            brightness: 0.0,
            sharpness: 0.0,
            gamma: 0.0,
            resolution: 0.0,
            artifacts: 0.0,
            fringing: 0.0,
            bleed: 0.0,
            merge_fields: true,
        }
    }
    // pub fn svideo() -> Self {
    //     Self {
    //         hue: 0.0,
    //         saturation: 0.0,
    //         contrast: 0.0,
    //         brightness: 0.0,
    //         sharpness: 0.2,
    //         gamma: 0.0,
    //         resolution: 0.2,
    //         artifacts: -1.0,
    //         fringing: -1.0,
    //         bleed: 0.0,
    //         merge_fields: true,
    //     }
    // }
}

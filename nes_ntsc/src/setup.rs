// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Image parameters, ranging from -1.0 to 1.0. Actual internal values shown
// in parenthesis and should remain fairly stable in future versions.
pub enum Setup {
    RGB,
    Composite,
    SVideo,
    MonoChrome,
    Custom {
        hue: f32,
        saturation: f32,
        contrast: f32,
        brightness: f32,
        sharpness: f32,
        gamma: f32,
        resolution: f32,
        artifacts: f32,
        fringing: f32,
        bleed: f32,
        merge_fields: bool,
    },
}

pub trait SetupValues {
    // Basic parameters
    fn hue(&self) -> f32; // -1 = -180 degrees     +1 = +180 degrees
    fn saturation(&self) -> f32; // -1 = grayscale (0.0)  +1 = oversaturated colors (2.0)
    fn contrast(&self) -> f32; // -1 = dark (0.5)       +1 = light (1.5)
    fn brightness(&self) -> f32; // -1 = dark (0.5)       +1 = light (1.5)
    fn sharpness(&self) -> f32; // edge contrast enhancement/blurring

    // Advanced parameters
    fn gamma(&self) -> f32; // -1 = dark (1.5)       +1 = light (0.5)
    fn resolution(&self) -> f32; // image resolution
    fn artifacts(&self) -> f32; // artifacts caused by color changes
    fn fringing(&self) -> f32; // color artifacts caused by brightness changes
    fn bleed(&self) -> f32; // color bleed (color resolution reduction)
    fn merge_fields(&self) -> bool; // if true, merges even and odd fields together to reduce flicker

    // decoder_matrix: &[f32; 6], // optional RGB decoder matrix, 6 elements

    // palette_out: &mut [u8], // optional RGB palette out, 3 bytes per color

    // // You can replace the standard NES color generation with an RGB palette. The
    // // first replaces all color generation, while the second replaces only the core
    // // 64-color generation and does standard color emphasis calculations on it.
    // palette: &[u8; 512 * 3], // optional 512-entry RGB palette in, 3 bytes per color
    // base_palette: &[u8; 64 * 3], // optional 64-entry RGB palette in, 3 bytes per color
}

impl SetupValues for Setup {
    fn hue(&self) -> f32 {
        match *self {
            Setup::RGB => 0.0,
            Setup::Composite => 0.0,
            Setup::SVideo => 0.0,
            Setup::MonoChrome => 0.0,
            Setup::Custom { hue, .. } => hue,
        }
    }

    fn saturation(&self) -> f32 {
        match *self {
            Setup::RGB => 0.0,
            Setup::Composite => 0.0,
            Setup::SVideo => 0.0,
            Setup::MonoChrome => -1.0,
            Setup::Custom { saturation, .. } => saturation,
        }
    }

    fn contrast(&self) -> f32 {
        match *self {
            Setup::RGB => 0.0,
            Setup::Composite => 0.0,
            Setup::SVideo => 0.0,
            Setup::MonoChrome => 0.0,
            Setup::Custom { contrast, .. } => contrast,
        }
    }

    fn brightness(&self) -> f32 {
        match *self {
            Setup::RGB => 0.0,
            Setup::Composite => 0.0,
            Setup::SVideo => 0.0,
            Setup::MonoChrome => 0.0,
            Setup::Custom { brightness, .. } => brightness,
        }
    }

    fn sharpness(&self) -> f32 {
        match *self {
            Setup::RGB => 0.2,
            Setup::Composite => 0.0,
            Setup::SVideo => 0.2,
            Setup::MonoChrome => 0.2,
            Setup::Custom { sharpness, .. } => sharpness,
        }
    }

    fn gamma(&self) -> f32 {
        match *self {
            Setup::RGB => 0.2,
            Setup::Composite => 0.0,
            Setup::SVideo => 0.0,
            Setup::MonoChrome => 0.0,
            Setup::Custom { gamma, .. } => gamma,
        }
    }

    fn resolution(&self) -> f32 {
        match *self {
            Setup::RGB => 0.7,
            Setup::Composite => 0.0,
            Setup::SVideo => 0.2,
            Setup::MonoChrome => 0.2,
            Setup::Custom { resolution, .. } => resolution,
        }
    }

    fn artifacts(&self) -> f32 {
        match *self {
            Setup::RGB => -1.0,
            Setup::Composite => 0.0,
            Setup::SVideo => -1.0,
            Setup::MonoChrome => -0.2,
            Setup::Custom { artifacts, .. } => artifacts,
        }
    }

    fn fringing(&self) -> f32 {
        match *self {
            Setup::RGB => -1.0,
            Setup::Composite => 0.0,
            Setup::SVideo => -1.0,
            Setup::MonoChrome => -0.2,
            Setup::Custom { fringing, .. } => fringing,
        }
    }

    fn bleed(&self) -> f32 {
        match *self {
            Setup::RGB => -1.0,
            Setup::Composite => 0.0,
            Setup::SVideo => 0.0,
            Setup::MonoChrome => -1.0,
            Setup::Custom { bleed, .. } => bleed,
        }
    }
    fn merge_fields(&self) -> bool {
        match *self {
            Setup::RGB => true,
            Setup::Composite => true,
            Setup::SVideo => true,
            Setup::MonoChrome => true,
            Setup::Custom { merge_fields, .. } => merge_fields,
        }
    }
}

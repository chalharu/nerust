// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod draw;
mod setup;

use crate::upload::FrameUploadLayout;
use nerust_screen_logical::LogicalSize;
use nerust_screen_physical::PhysicalSize;
use wgpu::{
    BindGroup, BindGroupLayout, Buffer, Device, Limits, Queue, RenderPipeline,
    SurfaceConfiguration, Texture,
};

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum RenderOutcome {
    Presented,
    Skipped,
    RecreateSurface,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresentationOptions {
    pub vsync: bool,
}

impl Default for PresentationOptions {
    fn default() -> Self {
        Self { vsync: true }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceLimitProfile {
    Default,
    DownlevelWebGl2,
}

impl DeviceLimitProfile {
    pub(crate) fn required_limits(self) -> Limits {
        match self {
            Self::Default => Limits::default(),
            Self::DownlevelWebGl2 => Limits::downlevel_webgl2_defaults(),
        }
    }
}

pub(crate) fn fit_surface_size_to_limit(
    surface_size: crate::surface::SurfaceSize,
    max_texture_dimension_2d: u32,
) -> crate::surface::SurfaceSize {
    let width = surface_size.width.max(1);
    let height = surface_size.height.max(1);
    let max_texture_dimension_2d = max_texture_dimension_2d.max(1);

    if width <= max_texture_dimension_2d && height <= max_texture_dimension_2d {
        return crate::surface::SurfaceSize::new(width, height);
    }

    let largest_dimension = width.max(height);
    let scaled_width = (u64::from(width) * u64::from(max_texture_dimension_2d)
        / u64::from(largest_dimension))
    .max(1) as u32;
    let scaled_height = (u64::from(height) * u64::from(max_texture_dimension_2d)
        / u64::from(largest_dimension))
    .max(1) as u32;
    crate::surface::SurfaceSize::new(scaled_width, scaled_height)
}

pub struct Renderer {
    device: Device,
    queue: Queue,
    config: SurfaceConfiguration,
    frame_texture: Texture,
    _palette_texture: Texture,
    _ntsc_texture: Texture,
    _srgb_lut_texture: Texture,
    frame_upload_buffer: Buffer,
    frame_upload_layout: FrameUploadLayout,
    frame_upload_staging: Box<[u8]>,
    _uniforms_buffer: Buffer,
    _bind_group_layout: BindGroupLayout,
    bind_group: BindGroup,
    pipeline: RenderPipeline,
    frame_logical_size: LogicalSize,
    content_size: PhysicalSize,
}

#[cfg(test)]
mod tests;

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
    BindGroup, BindGroupLayout, Buffer, Device, Queue, RenderPipeline, SurfaceConfiguration,
    Texture,
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

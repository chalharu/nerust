use super::{
    draw::compute_viewport,
    setup::{FramePipelineKind, composed_shader_source, encode_ntsc_texture, frame_logical_size},
};
use crate::surface::SurfaceSize;
use nerust_screen_filter::{FilterType, NTSC_TEXTURE_HEIGHT, NTSC_TEXTURE_WIDTH};
use nerust_screen_logical::LogicalSize;
use nerust_screen_physical::PhysicalSize;
use nerust_screen_video::{VideoFrameFormat, VideoFrameSpec, VideoPresentation};

#[test]
fn viewport_preserves_aspect_ratio() {
    let viewport = compute_viewport(
        SurfaceSize::new(1600, 900),
        PhysicalSize {
            width: 512.0,
            height: 480.0,
        },
    );

    assert_eq!(viewport.width, 960.0);
    assert_eq!(viewport.height, 900.0);
    assert_eq!(viewport.x, 320.0);
    assert_eq!(viewport.y, 0.0);
}

#[test]
fn ntsc_texture_is_prepacked_for_r32uint_upload() {
    let assets = FilterType::NtscRGB.palette_console_video_assets();
    let assets = assets
        .as_nes()
        .expect("NTSC filter should provide NES assets today");
    let source = assets
        .packed_ntsc_rgba8()
        .expect("NTSC filter should provide a packed texture");
    let (packed, size) = encode_ntsc_texture(Some(source));

    assert_eq!(size.width, NTSC_TEXTURE_WIDTH);
    assert_eq!(size.height, NTSC_TEXTURE_HEIGHT);
    assert_eq!(packed.len(), source.len());
    assert_eq!(
        &packed[..4],
        &u32::from_be_bytes(source[..4].try_into().expect("first texel must exist")).to_le_bytes()
    );
    assert_eq!(
        &packed[packed.len() - 4..],
        &u32::from_be_bytes(
            source[source.len() - 4..]
                .try_into()
                .expect("last texel must exist")
        )
        .to_le_bytes()
    );
}

#[test]
fn composed_shader_source_contains_split_stage_modules_once() {
    let source = composed_shader_source();
    assert_eq!(source.matches("fn fs_direct_linear").count(), 1);
    assert_eq!(source.matches("fn fs_direct_srgb").count(), 1);
    assert_eq!(source.matches("fn output_coords").count(), 1);
    assert_eq!(source.matches("fn palette_rgb_for_output").count(), 1);
    assert_eq!(source.matches("fn ntsc_rgb_for_output").count(), 1);
    assert_eq!(source.matches("fn fs_palette_linear").count(), 1);
}

#[test]
fn direct_color_upload_uses_logical_frame_size() {
    let presentation = VideoPresentation::new(VideoFrameSpec::new(
        VideoFrameFormat::Rgba,
        LogicalSize {
            width: 256,
            height: 240,
        },
        LogicalSize {
            width: 512,
            height: 480,
        },
        PhysicalSize {
            width: 512.0,
            height: 480.0,
        },
    ));

    let upload_size = frame_logical_size(&presentation, FramePipelineKind::DirectColor);

    assert_eq!(upload_size.width, 512);
    assert_eq!(upload_size.height, 480);
}

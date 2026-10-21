use nerust_render_filters::FilterTypeExt;
use nerust_render_ntsc::{NTSC_TEXTURE_HEIGHT, NTSC_TEXTURE_WIDTH};
use nerust_render_traits::{
    ReservedStrip, SurfaceSize, VideoFrameFormat, VideoFrameSpec, VideoPresentation,
    filter::FilterType, logical::LogicalSize, physical::PhysicalSize,
};

use super::{
    draw::compute_viewport,
    fit_surface_size_to_limit,
    setup::{composed_shader_source, encode_ntsc_texture},
};

#[test]
fn viewport_preserves_aspect_ratio() {
    let viewport = compute_viewport(
        SurfaceSize::new(1600, 900),
        PhysicalSize {
            width: 512.0,
            height: 480.0,
        },
        ReservedStrip::default(),
    );

    assert_eq!(viewport.width, 960.0);
    assert_eq!(viewport.height, 900.0);
    assert_eq!(viewport.x, 320.0);
    assert_eq!(viewport.y, 0.0);
}

#[test]
fn viewport_avoids_reserved_strips() {
    let viewport = compute_viewport(
        SurfaceSize::new(1600, 900),
        PhysicalSize {
            width: 512.0,
            height: 480.0,
        },
        ReservedStrip {
            top: 40,
            bottom: 60,
        },
    );

    // Available height 800: 512x480 fits at 800 high, 853 wide.
    assert_eq!(viewport.width, 853.333_3);
    assert_eq!(viewport.height, 800.0);
    assert_eq!(viewport.x, 373.333_34);
    assert_eq!(viewport.y, 40.0);
}

#[test]
fn viewport_stays_finite_for_extreme_aspects_and_full_reservation() {
    for value in [f32::MIN_POSITIVE, f32::from_bits(1), f32::MAX] {
        let viewport = compute_viewport(
            SurfaceSize::new(64, 64),
            PhysicalSize {
                width: value,
                height: value,
            },
            ReservedStrip::default(),
        );
        assert_eq!((viewport.width, viewport.height), (64.0, 64.0));
    }
    let viewport = compute_viewport(
        SurfaceSize::new(64, 64),
        PhysicalSize {
            width: 1.0,
            height: 1.0,
        },
        ReservedStrip {
            top: u32::MAX,
            bottom: u32::MAX,
        },
    );
    assert_eq!(
        (viewport.y, viewport.width, viewport.height),
        (64.0, 0.0, 0.0)
    );
}

#[test]
fn surface_size_is_scaled_down_without_distorting_aspect_ratio() {
    let fitted = fit_surface_size_to_limit(SurfaceSize::new(1080, 2356), 2048);

    assert_eq!(fitted, SurfaceSize::new(938, 2048));
    assert!(fitted.width <= 2048);
    assert!(fitted.height <= 2048);
    let source_ratio_scaled = u64::from(fitted.height) * 1080;
    let fitted_ratio_scaled = u64::from(fitted.width) * 2356;
    assert!(source_ratio_scaled >= fitted_ratio_scaled);
    assert!(source_ratio_scaled - fitted_ratio_scaled < 2356);
}

#[test]
fn surface_size_within_limit_is_unchanged() {
    let fitted = fit_surface_size_to_limit(SurfaceSize::new(1600, 900), 2048);

    assert_eq!(fitted, SurfaceSize::new(1600, 900));
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
fn composed_shader_source_avoids_gles_array_translation_gaps() {
    let source = composed_shader_source();
    assert!(!source.contains("array<array"));
    assert!(!source.contains("array<vec"));
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

    let upload_size = presentation.input_size();

    assert_eq!(upload_size.width, 512);
    assert_eq!(upload_size.height, 480);
}

#[test]
fn clipped_osd_rows_keep_original_source_pitch_and_clear_staging() {
    use super::setup::pack_osd_rect;
    let source: Vec<u8> = (0..48).collect(); // 4 pixels x 3 rows
    for (width, height) in [(1, 1), (2, 3), (4, 2), (4, 3)] {
        let mut staging = vec![255; 2048];
        let (pitch, len) = pack_osd_rect(&mut staging, width, height, Some((&source, 16))).unwrap();
        assert_eq!(len, pitch as usize * height as usize);
        for row in 0..height as usize {
            let start = row * pitch as usize;
            assert_eq!(
                &staging[start..start + width as usize * 4],
                &source[row * 16..row * 16 + width as usize * 4]
            );
            assert!(
                staging[start + width as usize * 4..start + pitch as usize]
                    .iter()
                    .all(|&b| b == 0)
            );
        }
        pack_osd_rect(&mut staging, width, height, None).unwrap();
        assert!(staging.iter().all(|&b| b == 0));
        staging.fill(255);
        assert!(pack_osd_rect(&mut staging, width, height, Some((&source[..1], 16))).is_none());
        assert!(staging.iter().all(|&b| b == 0));
    }
    assert!(pack_osd_rect(&mut Vec::new(), 0, 2, None).is_none());
}

#[test]
fn present_modes_honor_vsync_and_supported_fallbacks() {
    use wgpu::PresentMode::*;
    for (modes, vsync, expected) in [
        (vec![Immediate, Fifo], true, Fifo),
        (vec![Immediate, Fifo], false, Immediate),
        (vec![AutoNoVsync, AutoVsync, Fifo], true, AutoVsync),
        (vec![AutoNoVsync, AutoVsync, Fifo], false, AutoNoVsync),
        (vec![Fifo], false, Fifo),
        (vec![Mailbox, Fifo], false, Mailbox),
    ] {
        assert_eq!(
            super::RenderPipeline::select_present_mode(&modes, vsync),
            expected
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn device_osd_resize_recreates_texture_and_invalid_layers_clear_visibility() {
    use nerust_render_traits::renderer::OsdLayer;
    let _guard = crate::test_window::lock();
    let Some(window) = crate::test_window::TestWindow::open() else {
        return;
    };
    let instance = crate::surface::instance_with_display(window.clone());
    let (wh, dh) = window.handles();
    let surface = crate::surface::create_wgpu_surface(&instance, wh, dh).unwrap();
    let presentation = VideoPresentation::new(VideoFrameSpec::new(
        VideoFrameFormat::Rgba,
        LogicalSize {
            width: 4,
            height: 4,
        },
        LogicalSize {
            width: 4,
            height: 4,
        },
        PhysicalSize {
            width: 4.0,
            height: 4.0,
        },
    ));
    let mut pipeline = pollster::block_on(super::RenderPipeline::new(
        &instance,
        &surface,
        SurfaceSize::new(64, 64),
        &presentation,
        None,
        super::PresentationOptions { vsync: false },
        super::DeviceLimitProfile::Default,
    ))
    .unwrap();
    super::setup::set_osd_layer(
        &mut pipeline,
        Some(&OsdLayer::new(62, 3, 4, 2, (0..32).collect())),
    );
    assert_eq!(pipeline.osd_prev, Some((62, 3, 2, 2)));
    assert_eq!(
        &pipeline.osd_staging[256..264],
        &[16, 17, 18, 19, 20, 21, 22, 23]
    );
    pipeline.reconfigure_surface(&surface, SurfaceSize::new(96, 80));
    super::setup::set_osd_layer(
        &mut pipeline,
        Some(&OsdLayer::new(62, 3, 4, 2, (0..32).collect())),
    );
    let texture = pipeline.osd_texture.as_ref().unwrap();
    assert_eq!((texture.width(), texture.height()), (96, 80));
    assert!(pipeline.osd_visible);
    assert_eq!(pipeline.osd_prev, Some((62, 3, 4, 2)));
    pipeline.reconfigure_surface(&surface, SurfaceSize::new(16, 16));
    super::setup::set_osd_layer(
        &mut pipeline,
        Some(&OsdLayer::new(62, 3, 4, 2, (0..32).collect())),
    );
    assert!(!pipeline.osd_visible);
    pipeline.reconfigure_surface(&surface, SurfaceSize::new(96, 80));
    super::setup::set_osd_layer(
        &mut pipeline,
        Some(&OsdLayer::new(62, 3, 4, 2, (0..32).collect())),
    );
    assert!(pipeline.osd_visible);
    assert_eq!(pipeline.osd_prev, Some((62, 3, 4, 2)));
    for layer in [
        OsdLayer::new(62, 3, 4, 2, vec![255]),
        OsdLayer::new(0, 0, 0, 0, vec![]),
        OsdLayer::new(999, 999, 1, 1, vec![255; 4]),
    ] {
        super::setup::set_osd_layer(&mut pipeline, Some(&layer));
        assert!(!pipeline.osd_visible);
        assert!(pipeline.osd_prev.is_none());
    }
    pipeline
        .device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
}

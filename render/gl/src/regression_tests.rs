//! Pixel-level regressions driven by a real software-rendered pbuffer.
use crate::{frame_view::FrameView, osd_view::OsdView};
use nerust_glwrap::{Current, bindings as gl, raw};
use nerust_render_traits::{
    VideoFrameFormat, VideoRenderProfile, logical::LogicalSize, physical::PhysicalSize,
    renderer::OsdLayer,
};

fn profile() -> VideoRenderProfile {
    VideoRenderProfile {
        source_logical_size: LogicalSize {
            width: 64,
            height: 64,
        },
        logical_size: LogicalSize {
            width: 64,
            height: 64,
        },
        physical_size: PhysicalSize {
            width: 64.0,
            height: 64.0,
        },
        frame_format: VideoFrameFormat::Rgba,
        ntsc_packed_rgba8: None,
    }
}

fn framebuffer(w: i32, h: i32) -> Vec<u8> {
    raw::bind_buffer(gl::PIXEL_PACK_BUFFER, 0).unwrap();
    for (name, value) in [
        (gl::PACK_ALIGNMENT, 1),
        (gl::PACK_ROW_LENGTH, 0),
        (gl::PACK_SKIP_PIXELS, 0),
        (gl::PACK_SKIP_ROWS, 0),
    ] {
        raw::pixel_storei(name, value).unwrap();
    }
    let mut pixels = vec![0; (w * h * 4) as usize];
    unsafe {
        gl::ReadPixels(
            0,
            0,
            w,
            h,
            gl::RGBA,
            gl::UNSIGNED_BYTE,
            pixels.as_mut_ptr().cast(),
        )
    };
    nerust_glwrap::check_frame().unwrap();
    pixels
}

fn assert_pixels(pixels: &[u8], w: usize, h: usize, origin: Option<(usize, usize)>) {
    for y in 0..h {
        for x in 0..w {
            let expected = match origin {
                Some((ox, oy)) if (ox..ox + 2).contains(&x) && y == oy => [255, 0, 0, 255],
                Some((ox, oy)) if (ox..ox + 2).contains(&x) && y == oy + 1 => [0, 255, 0, 255],
                _ => [17, 31, 99, 255],
            };
            let offset = ((h - 1 - y) * w + x) * 4;
            assert_eq!(
                &pixels[offset..offset + 4],
                &expected,
                "pixel ({x}, {y}) on {w}x{h}"
            );
        }
    }
}

#[test]
fn final_pixels_preserve_osd_position_transparency_and_full_resize() {
    let _serial = crate::test_display::lock_display();
    let Some(display) = crate::test_display::TestDisplay::connect() else {
        return;
    };
    let _current = display
        .make_pbuffer_current(64, 64)
        .expect("pbuffer creation failed");
    let mut frame = FrameView::new();
    frame.set_use_vao(true);
    let is_gles = frame.load_frame(&profile()).unwrap();
    let mut osd = OsdView::new();
    osd.init_osd(is_gles);
    let background = [17, 31, 99, 255].repeat(64 * 64);
    let layer = |x, y| {
        OsdLayer::new(
            x,
            y,
            2,
            2,
            [255, 0, 0, 255]
                .repeat(2)
                .into_iter()
                .chain([0, 255, 0, 255].repeat(2))
                .collect(),
        )
    };

    // An unchanged layer must be reuploaded after both shrink and growth.
    // The high previous rectangle must not be cleared against a smaller store.
    for (size, origin, stage) in [
        (64, (3, 57), true),
        (32, (3, 5), true),
        (64, (3, 5), false),
        (32, (3, 5), false),
        (32, (9, 11), true),
    ] {
        if stage {
            osd.set_osd_layer(Some(layer(origin.0, origin.1)));
        }
        frame
            .apply_viewport(size, size, Default::default())
            .unwrap();
        frame.on_update(&background, 64).unwrap();
        osd.draw_osd(size, size, &mut frame, None).unwrap();
        assert_pixels(
            &framebuffer(size, size),
            size as usize,
            size as usize,
            Some((origin.0 as usize, origin.1 as usize)),
        );
        // Every unpainted texel of a newly allocated store is transparent.
        crate::frame_textures::use_texture(0, osd.osd_texture).unwrap();
        let mut texture = vec![0xEEu8; (size * size * 4) as usize];
        unsafe {
            raw::get_tex_image(
                gl::TEXTURE_2D,
                0,
                gl::RGBA,
                gl::UNSIGNED_BYTE,
                texture.as_mut_ptr().cast(),
            )
        }
        .unwrap();
        for (index, pixel) in texture.as_chunks::<4>().0.iter().enumerate() {
            let (x, y) = (index % size as usize, index / size as usize);
            if !(origin.0 as usize..origin.0 as usize + 2).contains(&x)
                || !(origin.1 as usize..origin.1 as usize + 2).contains(&y)
            {
                assert_eq!(pixel, &[0; 4]);
            }
        }
    }
    osd.set_osd_layer(None);
    frame.on_update(&background, 64).unwrap();
    osd.draw_osd(32, 32, &mut frame, None).unwrap();
    assert_pixels(&framebuffer(32, 32), 32, 32, None);
    let current = Current::assert_current(true);
    assert!(osd.close(current).is_empty());
    assert!(frame.close(current).is_empty());

    // Source dimensions are square, but the displayed pixels are not.
    let mut physical = profile();
    physical.physical_size.width = 32.0;
    frame.load_frame(&physical).unwrap();
    frame.apply_viewport(64, 64, Default::default()).unwrap();
    frame.on_update(&background, 64).unwrap();
    let pixels = framebuffer(64, 64);
    for row in pixels.as_chunks::<{ 64 * 4 }>().0 {
        for (x, pixel) in row.as_chunks::<4>().0.iter().enumerate() {
            let expected = if (16..48).contains(&x) {
                [17, 31, 99, 255]
            } else {
                [0, 0, 0, 255]
            };
            assert_eq!(pixel, &expected);
        }
    }
    assert!(frame.close(current).is_empty());
}

#[test]
fn shared_buffers_survive_array_teardown_and_init_failures_are_detected() {
    use nerust_glwrap::vertex::{Attrib, VertexArray, VertexBuffer};
    use std::rc::Rc;
    let _serial = crate::test_display::lock_display();
    let Some(display) = crate::test_display::TestDisplay::connect() else {
        return;
    };
    let _current = display
        .make_pbuffer_current(16, 16)
        .expect("pbuffer creation failed");
    let buffer = Rc::new(unsafe { VertexBuffer::from_slice(&[0.0f32; 16]) }.unwrap());
    let build = || {
        VertexArray::new(|init| {
            init.bind_vbo(buffer.clone(), |attrib| unsafe {
                attrib.attr_pointer(Attrib { id: 0 }, 2, gl::FLOAT, 16, 0)
            })
        })
        .unwrap()
    };
    let mut first = build();
    let mut second = build();
    let current = Current::assert_current(true);
    assert!(!first.destroy_if_current(current));
    assert_eq!(unsafe { gl::IsBuffer(buffer.id) }, gl::TRUE);
    assert!(!first.destroy_if_current(current));
    assert!(!second.destroy_if_current(current));
    assert_eq!(unsafe { gl::IsBuffer(buffer.id) }, gl::TRUE);
    assert!(!buffer.destroy_if_current(current));
    assert_eq!(unsafe { gl::IsBuffer(buffer.id) }, gl::FALSE);

    let buffer = Rc::new(unsafe { VertexBuffer::from_slice(&[0.0f32; 16]) }.unwrap());
    let id = buffer.id;
    assert!(
        VertexArray::new(
            |init| init.bind_vbo(buffer, |_| Err(nerust_glwrap::GlError::new(vec![
                nerust_glwrap::GlErrorKind::InvalidValue
            ])))
        )
        .is_err()
    );
    assert_eq!(unsafe { gl::IsBuffer(id) }, gl::FALSE);

    let buffer = Rc::new(unsafe { VertexBuffer::from_slice(&[0.0f32; 16]) }.unwrap());
    let id = buffer.id;
    let failure = VertexArray::new(|init| {
        unsafe { gl::BindBuffer(u32::MAX, 0) };
        init.bind_vbo(buffer, |_| Ok(()))
    });
    assert!(failure.is_err());
    assert_eq!(unsafe { gl::IsBuffer(id) }, gl::FALSE);

    let failure = VertexArray::new(|_| {
        // Bypass the debug drain: the constructor's final boundary must catch it in release too.
        unsafe { gl::BindBuffer(u32::MAX, 0) };
        Ok(())
    });
    assert!(failure.is_err());
    assert!(nerust_glwrap::check_frame().is_ok());
    unsafe { gl::BindBuffer(u32::MAX, 0) };
    assert!(raw::delete_buffers(1, &[0]).is_err());
    assert!(nerust_glwrap::check_frame().is_ok());
}

#[test]
fn metadata_and_unpadded_final_rows_are_validated_without_gl() {
    use nerust_render_traits::{FrameBuffer, PixelFormat};
    let size = LogicalSize {
        width: 2,
        height: 2,
    };
    let mut frame = FrameBuffer::with_capacity(2, 2, PixelFormat::Rgba);
    frame.resize(2, 2);
    frame.resize_data(frame.stride() + 8);
    assert!(crate::layout::validate_frame(&frame, size, VideoFrameFormat::Rgba, true).is_ok());
    assert!(crate::layout::validate_frame(&frame, size, VideoFrameFormat::Palette, true).is_err());
    assert!(
        crate::layout::validate_frame(
            &frame,
            LogicalSize {
                width: 3,
                height: 2
            },
            VideoFrameFormat::Rgba,
            true
        )
        .is_err()
    );
    assert!(
        crate::layout::FrameLayout::checked(
            LogicalSize {
                width: 0,
                height: 2
            },
            4,
            256,
            512,
            true
        )
        .is_err()
    );
    let mut invalid = profile();
    invalid.physical_size.width = f32::NAN;
    assert!(FrameView::new().load_frame(&invalid).is_err());
    invalid.physical_size.width = 64.0;
    invalid.logical_size.height = 0;
    assert!(FrameView::new().load_frame(&invalid).is_err());
}

#[test]
fn shared_frame_validation_maps_metadata_errors_on_every_path() {
    use crate::layout::{FrameError, validate_frame};
    use nerust_render_traits::{FrameBuffer, PixelFormat};
    let size = LogicalSize {
        width: 2,
        height: 2,
    };
    let mut frame = FrameBuffer::with_capacity(4, 4, PixelFormat::Rgba);
    frame.resize(4, 4);
    frame.resize_data(frame.stride() + 8);
    for unpack in [true, false] {
        let layout = validate_frame(&frame, size, VideoFrameFormat::Rgba, unpack).unwrap();
        assert_eq!((layout.width, layout.height, layout.need), (2, 2, 264));
        assert_eq!(
            validate_frame(&frame, size, VideoFrameFormat::Palette, unpack).err(),
            Some(FrameError::ProfileMismatch)
        );
        for size in [
            LogicalSize {
                width: 5,
                height: 2,
            },
            LogicalSize {
                width: 2,
                height: 5,
            },
        ] {
            assert_eq!(
                validate_frame(&frame, size, VideoFrameFormat::Rgba, unpack).err(),
                Some(FrameError::ProfileMismatch)
            );
        }
    }
    frame.resize_data(263);
    for unpack in [true, false] {
        assert_eq!(
            validate_frame(&frame, size, VideoFrameFormat::Rgba, unpack).err(),
            Some(FrameError::TooShort {
                need: 264,
                have: 263,
            })
        );
    }
}

#[test]
fn frame_textures_use_profile_input_size_for_rgba_and_palette() {
    let _serial = crate::test_display::lock_display();
    let Some(display) = crate::test_display::TestDisplay::connect() else {
        return;
    };
    let _current = display
        .make_pbuffer_current(16, 16)
        .expect("pbuffer creation failed");
    let mut profile = profile();
    profile.source_logical_size = LogicalSize {
        width: 2,
        height: 2,
    };
    profile.logical_size = LogicalSize {
        width: 4,
        height: 3,
    };
    for (format, expected) in [
        (VideoFrameFormat::Rgba, profile.logical_size),
        (VideoFrameFormat::Palette, profile.source_logical_size),
    ] {
        profile.frame_format = format;
        let mut frame = FrameView::new();
        frame.set_use_vao(true);
        frame.load_frame(&profile).unwrap();
        assert_eq!(profile.input_size(), expected);
        assert_eq!(frame.logical_width, expected.width as i32);
        assert_eq!(frame.logical_height, expected.height as i32);
        let bytes = vec![0u8; expected.width * expected.height * format.bytes_per_pixel()];
        frame.on_update(&bytes, expected.width as i32).unwrap();
        assert!(frame.close(Current::assert_current(true)).is_empty());
    }
}

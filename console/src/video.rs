use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use nerust_screen_video::FrameBuffer;
use nerust_screen_video::VideoRenderProfile;

#[derive(Debug)]
pub struct ConsoleVideo {
    render_profile: VideoRenderProfile,
    frame_buffer: Arc<Mutex<FrameBuffer>>,
    disp_fb: FrameBuffer,
    frame_ready: Arc<AtomicBool>,
}

impl ConsoleVideo {
    pub(crate) fn new(
        render_profile: VideoRenderProfile,
        frame_buffer: Arc<Mutex<FrameBuffer>>,
        disp_fb: FrameBuffer,
        frame_ready: Arc<AtomicBool>,
    ) -> Self {
        Self {
            render_profile,
            frame_buffer,
            disp_fb,
            frame_ready,
        }
    }

    pub fn render_profile(&self) -> VideoRenderProfile {
        self.render_profile.clone()
    }

    pub fn swap_frame_buffer(&mut self) {
        if !self.frame_ready.load(Ordering::Relaxed) {
            return;
        }
        if let Ok(mut guard) = self.frame_buffer.lock()
            && self.frame_ready.swap(false, Ordering::AcqRel)
        {
            std::mem::swap(&mut *guard, &mut self.disp_fb);
        }
    }

    pub fn frame_buffer(&self) -> &FrameBuffer {
        &self.disp_fb
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nerust_screen_video::{
        LogicalSize, PhysicalSize, PixelFormat, VideoFrameFormat, VideoRenderProfile,
    };

    fn make_test_video() -> ConsoleVideo {
        let mut shared = FrameBuffer::with_capacity(4, 1, PixelFormat::Rgba);
        shared.resize(4, 1);
        let mut disp = FrameBuffer::with_capacity(4, 1, PixelFormat::Rgba);
        disp.resize(4, 1);
        let shared = Arc::new(Mutex::new(shared));
        let frame_ready = Arc::new(AtomicBool::new(false));
        let profile = VideoRenderProfile {
            source_logical_size: LogicalSize {
                width: 4,
                height: 1,
            },
            logical_size: LogicalSize {
                width: 4,
                height: 1,
            },
            physical_size: PhysicalSize {
                width: 4.0,
                height: 1.0,
            },
            frame_format: VideoFrameFormat::Rgba,
            ntsc_packed_rgba8: None,
        };
        ConsoleVideo::new(profile, shared, disp, frame_ready)
    }

    #[test]
    fn console_video_swap_copies_shared_to_disp() {
        let mut video = make_test_video();
        {
            if let Ok(mut guard) = video.frame_buffer.lock() {
                guard.as_mut().fill(42);
            }
        }
        video.frame_ready.store(true, Ordering::Release);
        assert_eq!(video.frame_buffer().as_ref()[0], 0);
        video.swap_frame_buffer();
        assert_eq!(video.frame_buffer().as_ref()[0], 42);
    }
}

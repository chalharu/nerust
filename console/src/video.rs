use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use nerust_screen_video::LogicalSize;
use nerust_screen_video::PhysicalSize;
use nerust_screen_video::{FrameBuffer, VideoFrameFormat};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoFrameHandle {
    pub width: u32,
    pub height: u32,
    pub stride_bytes: usize,
    pub bytes: Arc<[u8]>,
}

impl VideoFrameHandle {
    pub fn bytes(&self) -> &[u8] {
        self.bytes.as_ref()
    }
}

#[derive(Debug, Clone)]
pub struct VideoRenderProfile {
    pub source_logical_size: LogicalSize,
    pub logical_size: LogicalSize,
    pub physical_size: PhysicalSize,
    pub frame_format: VideoFrameFormat,
    pub ntsc_packed_rgba8: Option<Box<[u8]>>,
}

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

    pub fn with_frame_buffer<T>(&self, f: impl FnOnce(&[u8]) -> T) -> T {
        f(self.disp_fb.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nerust_screen_video::{PixelFormat, VideoFrameFormat};

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
        // Simulate EmuThread signaling frame ready
        video.frame_ready.store(true, Ordering::Release);
        video.with_frame_buffer(|bytes| assert_eq!(bytes[0], 0));
        video.swap_frame_buffer();
        video.with_frame_buffer(|bytes| assert_eq!(bytes[0], 42));
    }
}

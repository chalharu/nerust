use std::sync::{Arc, Mutex};

use nerust_screen_logical::LogicalSize;
use nerust_screen_physical::PhysicalSize;
use nerust_screen_video::FrameBuffer;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoFrameHandle {
    pub width: u32,
    pub height: u32,
    pub stride_bytes: usize,
    bytes: Arc<[u8]>,
}

impl VideoFrameHandle {
    pub fn bytes(&self) -> &[u8] {
        self.bytes.as_ref()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct VideoRenderProfile {
    pub source_logical_size: LogicalSize,
    pub logical_size: LogicalSize,
    pub physical_size: PhysicalSize,
}

#[derive(Debug, Clone)]
pub struct ConsoleVideo {
    render_profile: VideoRenderProfile,
    frame_buffer: Arc<Mutex<FrameBuffer>>,
}

impl ConsoleVideo {
    pub(crate) fn new(
        render_profile: VideoRenderProfile,
        frame_buffer: Arc<Mutex<FrameBuffer>>,
    ) -> Self {
        Self {
            render_profile,
            frame_buffer,
        }
    }

    pub fn render_profile(&self) -> VideoRenderProfile {
        self.render_profile
    }

    pub fn with_frame_buffer<T>(&self, f: impl FnOnce(&[u8]) -> T) -> T {
        let guard = self.frame_buffer.lock().unwrap();
        f(guard.as_ref())
    }

    pub fn frame_handle(&self) -> VideoFrameHandle {
        let guard = self.frame_buffer.lock().unwrap();
        VideoFrameHandle {
            width: guard.width() as u32,
            height: guard.height() as u32,
            stride_bytes: guard.stride(),
            bytes: Arc::from(guard.as_ref()),
        }
    }
}

/// NullAudio を返すダミーの AudioBackend。テスト用。
#[cfg(test)]
mod tests {
    use super::*;
    use nerust_screen_video::{FrameBuffer, PixelFormat};
    use std::sync::{Arc, Mutex};

    #[test]
    fn console_video_reads_frame_buffer() {
        let mut fb = FrameBuffer::with_capacity(4, 1, PixelFormat::Rgba);
        fb.resize(4, 1);
        // Rgba stride = ((4*4).max(1)+255)&!255 = 256
        let data = vec![1u8; 256];
        fb.as_mut().copy_from_slice(&data);
        let shared = Arc::new(Mutex::new(fb));
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
        };
        let video = ConsoleVideo::new(profile, shared.clone());

        video.with_frame_buffer(|bytes| {
            assert_eq!(bytes.len(), 256);
            assert_eq!(bytes[0], 1);
        });
    }
}

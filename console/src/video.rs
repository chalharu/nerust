use nerust_screen_logical::LogicalSize;
use nerust_screen_physical::PhysicalSize;
use nerust_screen_video::VideoPresentation;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone)]
pub struct VideoFrameBuffer(Arc<RwLock<Box<[u8]>>>);

impl VideoFrameBuffer {
    fn from_shared(shared: Arc<RwLock<Box<[u8]>>>) -> Self {
        Self(shared)
    }

    pub fn with_bytes<T>(&self, f: impl FnOnce(&[u8]) -> T) -> T {
        let bytes = self.0.read().unwrap_or_else(|err| err.into_inner());
        f(bytes.as_ref())
    }

    fn snapshot(&self) -> Arc<[u8]> {
        let bytes = self.0.read().unwrap_or_else(|err| err.into_inner());
        Arc::from(bytes.as_ref())
    }
}

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

#[derive(Debug, Clone, Copy)]
pub struct VideoRenderProfile {
    pub source_logical_size: LogicalSize,
    pub logical_size: LogicalSize,
    pub physical_size: PhysicalSize,
}

#[derive(Debug, Clone)]
pub struct ConsoleVideo {
    render_profile: VideoRenderProfile,
    frame_width: u32,
    frame_height: u32,
    stride_bytes: usize,
    frame_buffer: VideoFrameBuffer,
}

impl ConsoleVideo {
    pub(crate) fn new(
        presentation: VideoPresentation,
        frame_buffer: Arc<RwLock<Box<[u8]>>>,
    ) -> Self {
        let logical_size = presentation.logical_size();
        Self {
            render_profile: VideoRenderProfile {
                source_logical_size: presentation.source_logical_size(),
                logical_size,
                physical_size: presentation.physical_size(),
            },
            frame_width: logical_size.width as u32,
            frame_height: logical_size.height as u32,
            stride_bytes: logical_size.width * 4,
            frame_buffer: VideoFrameBuffer::from_shared(frame_buffer),
        }
    }

    pub fn render_profile(&self) -> VideoRenderProfile {
        self.render_profile
    }

    pub fn with_frame_buffer<T>(&self, f: impl FnOnce(&[u8]) -> T) -> T {
        self.frame_buffer.with_bytes(f)
    }

    pub fn frame_handle(&self) -> VideoFrameHandle {
        VideoFrameHandle {
            width: self.frame_width,
            height: self.frame_height,
            stride_bytes: self.stride_bytes,
            bytes: self.frame_buffer.snapshot(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::VideoFrameBuffer;
    use std::sync::{Arc, RwLock};

    #[test]
    fn video_frame_buffer_supports_shared_reads() {
        let shared = Arc::new(RwLock::new(vec![1, 2, 3].into_boxed_slice()));
        let buffer = VideoFrameBuffer::from_shared(shared.clone());

        {
            let mut bytes = shared.write().unwrap_or_else(|err| err.into_inner());
            bytes[1] = 9;
        }

        buffer.with_bytes(|bytes| assert_eq!(bytes, [1, 9, 3]));
    }
}

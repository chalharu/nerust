use nerust_screen_buffer::video_api::{ConsoleVideoAssets, VideoPresentation};
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
}

#[derive(Debug, Clone)]
pub struct ConsoleVideo {
    presentation: VideoPresentation,
    console_video_assets: Option<ConsoleVideoAssets>,
    frame_buffer: VideoFrameBuffer,
}

impl ConsoleVideo {
    pub(crate) fn new(
        presentation: VideoPresentation,
        console_video_assets: Option<ConsoleVideoAssets>,
        frame_buffer: Arc<RwLock<Box<[u8]>>>,
    ) -> Self {
        Self {
            presentation,
            console_video_assets,
            frame_buffer: VideoFrameBuffer::from_shared(frame_buffer),
        }
    }

    pub fn presentation(&self) -> &VideoPresentation {
        &self.presentation
    }

    pub fn frame_buffer(&self) -> &VideoFrameBuffer {
        &self.frame_buffer
    }

    pub fn console_video_assets(&self) -> Option<&ConsoleVideoAssets> {
        self.console_video_assets.as_ref()
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

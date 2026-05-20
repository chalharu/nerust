use nerust_screen_filter::presentation::VideoPresentation;
use nerust_screen_traits::video_frame::VideoFrameBuffer;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone)]
pub struct ConsoleVideo {
    presentation: VideoPresentation,
    frame_buffer: VideoFrameBuffer,
}

impl ConsoleVideo {
    pub(crate) fn new(
        presentation: VideoPresentation,
        frame_buffer: Arc<RwLock<Box<[u8]>>>,
    ) -> Self {
        Self {
            presentation,
            frame_buffer: VideoFrameBuffer::from_shared(frame_buffer),
        }
    }

    pub fn presentation(&self) -> &VideoPresentation {
        &self.presentation
    }

    pub fn frame_buffer(&self) -> &VideoFrameBuffer {
        &self.frame_buffer
    }
}

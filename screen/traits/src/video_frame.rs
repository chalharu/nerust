use crate::{LogicalSize, PhysicalSize};
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum VideoFrameFormat {
    Rgba,
    Palette,
}

#[derive(Debug, Clone, Copy)]
pub struct VideoFrameSpec {
    frame_format: VideoFrameFormat,
    source_logical_size: LogicalSize,
    logical_size: LogicalSize,
    physical_size: PhysicalSize,
}

impl VideoFrameSpec {
    pub fn new(
        frame_format: VideoFrameFormat,
        source_logical_size: LogicalSize,
        logical_size: LogicalSize,
        physical_size: PhysicalSize,
    ) -> Self {
        Self {
            frame_format,
            source_logical_size,
            logical_size,
            physical_size,
        }
    }

    pub fn frame_format(&self) -> VideoFrameFormat {
        self.frame_format
    }

    pub fn source_logical_size(&self) -> LogicalSize {
        self.source_logical_size
    }

    pub fn logical_size(&self) -> LogicalSize {
        self.logical_size
    }

    pub fn physical_size(&self) -> PhysicalSize {
        self.physical_size
    }
}

#[derive(Debug, Clone)]
pub struct VideoFrameBuffer(Arc<RwLock<Box<[u8]>>>);

impl VideoFrameBuffer {
    pub fn new(initial: Box<[u8]>) -> Self {
        Self::from_shared(Arc::new(RwLock::new(initial)))
    }

    pub fn from_shared(shared: Arc<RwLock<Box<[u8]>>>) -> Self {
        Self(shared)
    }

    pub fn with_bytes<T>(&self, f: impl FnOnce(&[u8]) -> T) -> T {
        let bytes = self.0.read().unwrap_or_else(|err| err.into_inner());
        f(bytes.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::{VideoFrameBuffer, VideoFrameFormat, VideoFrameSpec};
    use crate::{LogicalSize, PhysicalSize};
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

    #[test]
    fn video_frame_spec_accessors_match_constructor_inputs() {
        let spec = VideoFrameSpec::new(
            VideoFrameFormat::Palette,
            LogicalSize {
                width: 256,
                height: 240,
            },
            LogicalSize {
                width: 602,
                height: 240,
            },
            PhysicalSize {
                width: 602.0,
                height: 480.0,
            },
        );

        assert_eq!(spec.frame_format(), VideoFrameFormat::Palette);
        let source_logical_size = spec.source_logical_size();
        assert_eq!(source_logical_size.width, 256);
        assert_eq!(source_logical_size.height, 240);

        let logical_size = spec.logical_size();
        assert_eq!(logical_size.width, 602);
        assert_eq!(logical_size.height, 240);

        let physical_size = spec.physical_size();
        assert_eq!(physical_size.width, 602.0);
        assert_eq!(physical_size.height, 480.0);
    }
}

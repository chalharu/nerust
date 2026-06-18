use super::screen_buffer_unit::ScreenBufferUnit;
use nerust_screen_filter::presentation::ConsoleVideoAssets;
use nerust_screen_filter::{BLACK_PALETTE_INDEX, FilterType, NesFilter};
use nerust_screen_video::LogicalSize;
use nerust_screen_video::PhysicalSize;
use nerust_screen_video::{FrameBuffer, PixelFormat, Screen, VideoPresentation};
use std::hash::{Hash, Hasher};
use std::mem;

const DEFAULT_NES_FILTER_TYPE: FilterType = FilterType::NtscComposite;
const DEFAULT_NES_SOURCE_LOGICAL_SIZE: LogicalSize = LogicalSize {
    width: 256,
    height: 240,
};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum PublishMode {
    FilteredRgba,
    SourcePalette,
}

pub struct ScreenBuffer {
    filter_type: FilterType,
    video_presentation: VideoPresentation,
    console_video_assets: Option<ConsoleVideoAssets>,
    publish_mode: PublishMode,
    filter: Option<Box<dyn NesFilter>>,
    dest: Option<ScreenBufferUnit>,
    display_buffer: Option<ScreenBufferUnit>,
    back: FrameBuffer,
    front: FrameBuffer,
    src_pos: usize,
}

impl ScreenBuffer {
    pub fn new(filter_type: FilterType, src_size: LogicalSize) -> Self {
        Self::with_publish_mode(
            filter_type,
            src_size,
            PublishMode::FilteredRgba,
            filter_type.rgba_presentation(src_size),
            None,
        )
    }

    pub fn new_gpu(filter_type: FilterType, src_size: LogicalSize) -> Self {
        let video_presentation = filter_type.palette_presentation(src_size);
        Self::with_publish_mode(
            filter_type,
            src_size,
            PublishMode::SourcePalette,
            video_presentation,
            Some(filter_type.palette_console_video_assets()),
        )
    }

    pub fn new_nes_gpu_default() -> Self {
        Self::new_gpu(DEFAULT_NES_FILTER_TYPE, DEFAULT_NES_SOURCE_LOGICAL_SIZE)
    }

    fn with_publish_mode(
        filter_type: FilterType,
        src_size: LogicalSize,
        publish_mode: PublishMode,
        video_presentation: VideoPresentation,
        console_video_assets: Option<ConsoleVideoAssets>,
    ) -> Self {
        let palette = Box::new([0u32; 256]);
        let mut back = FrameBuffer::with_capacity(
            src_size.width,
            src_size.height,
            PixelFormat::PaletteIndex {
                palette: palette.clone(),
            },
        );
        let mut front = FrameBuffer::with_capacity(
            src_size.width,
            src_size.height,
            PixelFormat::PaletteIndex { palette },
        );
        back.resize(src_size.width, src_size.height);
        front.resize(src_size.width, src_size.height);
        let (filter, dest, display_buffer) = match publish_mode {
            PublishMode::FilteredRgba => {
                let filter = filter_type.generate(src_size);
                let logical_size = filter.logical_size();
                (
                    Some(filter),
                    Some(ScreenBufferUnit::new(logical_size)),
                    Some(ScreenBufferUnit::new(logical_size)),
                )
            }
            PublishMode::SourcePalette => (None, None, None),
        };

        let mut result = Self {
            filter_type,
            video_presentation,
            console_video_assets,
            publish_mode,
            filter,
            dest,
            display_buffer,
            back,
            front,
            src_pos: 0,
        };
        result.clear();
        result
    }

    pub fn frame_len(&self) -> usize {
        match self.publish_mode {
            PublishMode::FilteredRgba => self
                .display_buffer
                .as_ref()
                .expect("filtered buffers should exist for CPU screen buffers")
                .byte_len(),
            PublishMode::SourcePalette => self.front.width() * self.front.height(),
        }
    }

    pub fn source_frame_len(&self) -> usize {
        self.front.width() * self.front.height()
    }

    pub fn copy_source_buffer(&self, dest: &mut [u8]) {
        let pixel_bytes = self.front.width() * self.front.height();
        assert_eq!(dest.len(), pixel_bytes, "source buffer size mismatch");
        dest.copy_from_slice(&self.front.as_ref()[..pixel_bytes]);
    }

    /// Write the current displayable frame into `dest`.
    /// For FilteredRgba mode this writes RGBA bytes (4 bytes/pixel).
    /// For SourcePalette mode this writes palette indices (1 byte/pixel).
    pub fn write_frame_into(&self, dest: &mut [u8]) {
        match self.publish_mode {
            PublishMode::FilteredRgba => self
                .display_buffer
                .as_ref()
                .expect("filtered buffers should exist for CPU screen buffers")
                .copy_to_slice(dest),
            PublishMode::SourcePalette => {
                let pixel_bytes = self.front.width() * self.front.height();
                dest.copy_from_slice(&self.front.as_ref()[..pixel_bytes]);
            }
        }
    }

    pub fn front_frame(&self) -> &FrameBuffer {
        &self.front
    }

    pub fn logical_size(&self) -> LogicalSize {
        self.video_presentation.logical_size()
    }

    pub fn source_logical_size(&self) -> LogicalSize {
        self.video_presentation.source_logical_size()
    }

    pub fn filter_type(&self) -> FilterType {
        self.filter_type
    }

    pub fn physical_size(&self) -> PhysicalSize {
        self.video_presentation.physical_size()
    }

    pub fn publishes_palette_frame(&self) -> bool {
        matches!(self.publish_mode, PublishMode::SourcePalette)
    }

    pub fn video_presentation(&self) -> &VideoPresentation {
        &self.video_presentation
    }

    pub fn console_video_assets(&self) -> Option<&ConsoleVideoAssets> {
        self.console_video_assets.as_ref()
    }

    pub fn clear(&mut self) {
        if let Some(dest) = self.dest.as_mut() {
            dest.clear();
        }
        if let Some(display_buffer) = self.display_buffer.as_mut() {
            display_buffer.clear();
        }
        let pixel_bytes = self.front.width() * self.front.height();
        self.front.as_mut()[..pixel_bytes].fill(BLACK_PALETTE_INDEX);
        self.back.as_mut()[..pixel_bytes].fill(BLACK_PALETTE_INDEX);
        self.src_pos = 0;
    }

    pub fn restore_source_buffer(&mut self, source: &[u8]) {
        assert!(
            self.publishes_palette_frame(),
            "source buffer restore is only supported for palette-published screen buffers"
        );
        let pixel_bytes = self.front.width() * self.front.height();
        assert_eq!(
            source.len(),
            pixel_bytes,
            "source buffer size mismatch during restore"
        );
        self.front.as_mut()[..pixel_bytes].copy_from_slice(source);
        self.back.as_mut()[..pixel_bytes].copy_from_slice(source);
        self.src_pos = 0;
    }
}

impl Screen for ScreenBuffer {
    fn push(&mut self, value: u8) {
        if let (Some(filter), Some(dest)) = (self.filter.as_mut(), self.dest.as_mut()) {
            filter.push(value, dest);
        }
        self.back.as_mut()[self.src_pos] = value;
        self.src_pos += 1;
    }

    #[inline]
    fn push_many(&mut self, value: u8, count: u16) {
        let count = usize::from(count);
        if let (Some(filter), Some(dest)) = (self.filter.as_mut(), self.dest.as_mut()) {
            for _ in 0..count {
                filter.push(value, dest);
            }
        }
        let end = self.src_pos + count;
        self.back.as_mut()[self.src_pos..end].fill(value);
        self.src_pos = end;
    }

    fn render(&mut self) {
        let pixel_count = self.back.width() * self.back.height();
        assert_eq!(
            self.src_pos, pixel_count,
            "source frame size mismatch before publish"
        );
        mem::swap(&mut self.back, &mut self.front);
        self.src_pos = 0;
        if let (Some(dest), Some(display_buffer)) =
            (self.dest.as_mut(), self.display_buffer.as_mut())
        {
            assert_eq!(
                dest.pixel_len(),
                display_buffer.pixel_len(),
                "display buffer sizes diverged"
            );
            assert!(
                dest.is_full(),
                "filtered frame size mismatch before publish"
            );
            mem::swap(dest, display_buffer);
            dest.reset();
        }
    }
}

impl Hash for ScreenBuffer {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.front.as_ref().hash(state);
    }
}

#[cfg(test)]
mod tests {
    use super::ScreenBuffer;
    use nerust_screen_filter::FilterType;
    use nerust_screen_video::LogicalSize;
    use nerust_screen_video::{Screen, VideoFrameFormat};

    #[test]
    fn all_filters_publish_full_frames() {
        let source = LogicalSize {
            width: 256,
            height: 240,
        };
        for filter in [
            FilterType::None,
            FilterType::NtscRGB,
            FilterType::NtscComposite,
            FilterType::NtscSVideo,
        ] {
            let mut screen = ScreenBuffer::new(filter, source);
            for _ in 0..(source.width * source.height) {
                screen.push(0);
            }
            screen.render();
        }
    }

    #[test]
    fn gpu_screen_buffer_publishes_palette_frames() {
        let source = LogicalSize {
            width: 4,
            height: 2,
        };
        let mut screen = ScreenBuffer::new_gpu(FilterType::NtscComposite, source);
        for value in 0..(source.width * source.height) {
            screen.push(value as u8);
        }
        screen.render();

        let mut frame = vec![0; screen.frame_len()];
        screen.write_frame_into(&mut frame);
        assert_eq!(frame, (0..8).map(|value| value as u8).collect::<Vec<_>>());
        assert_eq!(
            screen.video_presentation().frame_format(),
            VideoFrameFormat::Palette
        );
        assert_eq!(
            screen
                .console_video_assets()
                .map(|assets| assets.as_nes().unwrap().pipeline_kind()),
            Some(nerust_screen_filter::presentation::VideoPresentationPipelineKind::Ntsc)
        );
    }

    #[test]
    fn default_nes_gpu_screen_buffer_uses_standard_source_size() {
        let screen = ScreenBuffer::new_nes_gpu_default();

        assert!(screen.publishes_palette_frame());
        assert!(matches!(screen.filter_type(), FilterType::NtscComposite));
        assert_eq!(screen.source_logical_size().width, 256);
        assert_eq!(screen.source_logical_size().height, 240);
    }
}

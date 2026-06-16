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

#[derive(Debug)]
pub struct ConsoleVideo {
    render_profile: VideoRenderProfile,
    frame_buffer: Arc<Mutex<FrameBuffer>>,
    /// 表示バッファ（GUI スレッドローカル）。
    /// `swap_frame_buffer(&mut self)` で shared から最新フレームを引き取り、
    /// `with_frame_buffer(&self)` でロックなし読み取り。
    disp_fb: FrameBuffer,
}

impl ConsoleVideo {
    pub(crate) fn new(
        render_profile: VideoRenderProfile,
        frame_buffer: Arc<Mutex<FrameBuffer>>,
        disp_fb: FrameBuffer,
    ) -> Self {
        Self {
            render_profile,
            frame_buffer,
            disp_fb,
        }
    }

    pub fn render_profile(&self) -> VideoRenderProfile {
        self.render_profile
    }

    /// 共有バッファから表示バッファに最新フレームを引き取る（`&mut self`）。
    /// GUI スレッドが各フレームの描画前に1回呼ぶ。
    pub fn swap_frame_buffer(&mut self) {
        let mut guard = self.frame_buffer.lock().unwrap();
        std::mem::swap(&mut *guard, &mut self.disp_fb);
    }

    /// 表示バッファの内容をロックなしで読み取る。
    /// `swap_frame_buffer()` の後に呼ぶこと。
    pub fn with_frame_buffer<T>(&self, f: impl FnOnce(&[u8]) -> T) -> T {
        f(self.disp_fb.as_ref())
    }

    /// 共有バッファの内容を直接読み取る（ロックあり）。
    pub fn read_shared<T>(&self, f: impl FnOnce(&[u8]) -> T) -> T {
        let guard = self.frame_buffer.lock().unwrap();
        f(guard.as_ref())
    }

    /// VideoFrameHandle を生成する。
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

#[cfg(test)]
mod tests {
    use super::*;
    use nerust_screen_video::{FrameBuffer, PixelFormat};

    fn make_test_video() -> ConsoleVideo {
        let mut shared = FrameBuffer::with_capacity(4, 1, PixelFormat::Rgba);
        shared.resize(4, 1);
        let mut disp = FrameBuffer::with_capacity(4, 1, PixelFormat::Rgba);
        disp.resize(4, 1);
        let shared = Arc::new(Mutex::new(shared));
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
        ConsoleVideo::new(profile, shared, disp)
    }

    #[test]
    fn console_video_swap_brings_new_data() {
        let mut video = make_test_video();
        {
            let mut guard = video.frame_buffer.lock().unwrap();
            guard.as_mut().fill(42);
        }
        video.with_frame_buffer(|bytes| {
            assert_eq!(bytes[0], 0);
        });
        video.swap_frame_buffer();
        video.with_frame_buffer(|bytes| {
            assert_eq!(bytes[0], 42);
        });
    }

    #[test]
    fn console_video_frame_handle_reads_shared() {
        let video = make_test_video();
        {
            let mut guard = video.frame_buffer.lock().unwrap();
            guard.as_mut().fill(99);
        }
        let handle = video.frame_handle();
        assert_eq!(handle.bytes()[0], 99);
        assert_eq!(handle.width, 4);
        assert_eq!(handle.stride_bytes, 256);
    }
}

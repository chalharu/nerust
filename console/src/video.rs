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
    /// 表示バッファ。`swap_frame_buffer()` で shared から最新フレームを引き取る。
    /// `with_frame_buffer()` はこのバッファを読む（shared と別の Mutex）。
    disp_fb: Mutex<FrameBuffer>,
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
            disp_fb: Mutex::new(disp_fb),
        }
    }

    pub fn render_profile(&self) -> VideoRenderProfile {
        self.render_profile
    }

    /// 共有バッファから表示バッファに最新フレームを引き取る（`&self` で呼び出し可能）。
    /// GUI スレッドが各フレームの描画前に1回呼ぶ。
    pub fn swap_frame_buffer(&self) {
        let mut guard = self.frame_buffer.lock().unwrap();
        let mut disp = self.disp_fb.lock().unwrap();
        std::mem::swap(&mut *guard, &mut *disp);
    }

    /// 表示バッファの内容を読み取る（disp_fb の Mutex をロック）。
    /// shared とは別の Mutex なので Console スレッドとの競合は無視できる。
    pub fn with_frame_buffer<T>(&self, f: impl FnOnce(&[u8]) -> T) -> T {
        let disp = self.disp_fb.lock().unwrap();
        f(disp.as_ref())
    }

    /// 共有バッファの内容を直接読み取る。セーブステート等 `swap` 不要な場合に使用。
    pub fn read_shared<T>(&self, f: impl FnOnce(&[u8]) -> T) -> T {
        let guard = self.frame_buffer.lock().unwrap();
        f(guard.as_ref())
    }

    /// swap + 読み取りを一度に行う。
    pub fn swap_and_read<T>(&self, f: impl FnOnce(&[u8]) -> T) -> T {
        self.swap_frame_buffer();
        self.with_frame_buffer(f)
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
        let video = make_test_video();
        {
            let mut guard = video.frame_buffer.lock().unwrap();
            guard.as_mut().fill(42);
        }
        // Before swap: disp_fb is empty (zeros)
        video.with_frame_buffer(|bytes| {
            assert_eq!(bytes[0], 0);
        });
        // swap_frame_buffer takes &self now
        video.swap_frame_buffer();
        video.with_frame_buffer(|bytes| {
            assert_eq!(bytes[0], 42);
        });
    }

    #[test]
    fn console_video_swap_and_read_combines_both() {
        let video = make_test_video();
        {
            let mut guard = video.frame_buffer.lock().unwrap();
            guard.as_mut().fill(77);
        }
        video.swap_and_read(|bytes| {
            assert_eq!(bytes[0], 77);
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

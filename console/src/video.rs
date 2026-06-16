use std::sync::{Arc, Mutex, atomic::AtomicBool};

use nerust_screen_logical::LogicalSize;
use nerust_screen_physical::PhysicalSize;
use nerust_screen_video::FrameBuffer;

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

#[derive(Debug)]
pub struct ConsoleVideo {
    render_profile: VideoRenderProfile,
    frame_buffer: Arc<Mutex<FrameBuffer>>,
    /// 表示バッファ（GUI スレッドローカル）。
    /// `swap_frame_buffer(&mut self)` で shared から最新フレームを引き取り、
    /// `with_frame_buffer(&self)` でロックなし読み取り。
    disp_fb: FrameBuffer,
    /// 共有バッファに新しいフレームが書き込まれたかどうか。
    /// GUI スレッドが各フレームの描画前に1回チェックする。
    frame_buffer_updated: Arc<AtomicBool>,
}

impl ConsoleVideo {
    pub(crate) fn new(
        render_profile: VideoRenderProfile,
        frame_buffer: Arc<Mutex<FrameBuffer>>,
        disp_fb: FrameBuffer,
        frame_buffer_updated: Arc<AtomicBool>,
    ) -> Self {
        Self {
            render_profile,
            frame_buffer,
            disp_fb,
            frame_buffer_updated,
        }
    }

    pub fn render_profile(&self) -> VideoRenderProfile {
        self.render_profile
    }

    /// 共有バッファから表示バッファに最新フレームを引き取る（`&mut self`）。
    /// GUI スレッドが各フレームの描画前に1回呼ぶ。
    pub fn swap_frame_buffer(&mut self) {
        if self
            .frame_buffer_updated
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            let mut guard = self.frame_buffer.lock().unwrap();
            if self
                .frame_buffer_updated
                .swap(false, std::sync::atomic::Ordering::AcqRel)
            {
                std::mem::swap(&mut *guard, &mut self.disp_fb);
            }
        }
    }

    /// 表示バッファの内容をロックなしで読み取る。
    /// `swap_frame_buffer()` の後に呼ぶこと。
    pub fn with_frame_buffer<T>(&self, f: impl FnOnce(&[u8]) -> T) -> T {
        f(self.disp_fb.as_ref())
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
        ConsoleVideo::new(profile, shared, disp, Arc::new(AtomicBool::new(false)))
    }

    #[test]
    fn console_video_swap_brings_new_data() {
        let mut video = make_test_video();
        {
            let mut guard = video.frame_buffer.lock().unwrap();
            guard.as_mut().fill(42);
            video
                .frame_buffer_updated
                .store(true, std::sync::atomic::Ordering::Release);
        }
        video.with_frame_buffer(|bytes| {
            assert_eq!(bytes[0], 0);
        });
        video.swap_frame_buffer();
        video.with_frame_buffer(|bytes| {
            assert_eq!(bytes[0], 42);
        });
    }
}

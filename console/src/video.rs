use std::sync::{Arc, Mutex};

use nerust_contract_core::GpuCommand;
use nerust_contract_core::channel::{EmuToRenderer, FrameChannelRenderer};
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
    /// Renderer 側のチャネルハンドル。コマンド受信と ACK 送信を行う。
    renderer_channel: FrameChannelRenderer,
    /// `swap_frame_buffer()` で `UploadPalette` を受信した場合に true にセット。
    palette_updated: bool,
}

impl ConsoleVideo {
    pub(crate) fn new(
        render_profile: VideoRenderProfile,
        frame_buffer: Arc<Mutex<FrameBuffer>>,
        disp_fb: FrameBuffer,
        renderer_channel: FrameChannelRenderer,
    ) -> Self {
        Self {
            render_profile,
            frame_buffer,
            disp_fb,
            renderer_channel,
            palette_updated: false,
        }
    }

    pub fn render_profile(&self) -> VideoRenderProfile {
        self.render_profile
    }

    /// 共有バッファから表示バッファに最新フレームを引き取る（`&mut self`）。
    /// GUI スレッドが各フレームの描画前に1回呼ぶ。
    /// `UploadPalette` を受信した場合、`palette_updated()` が true を返す。
    pub fn swap_frame_buffer(&mut self) {
        if let Some(EmuToRenderer::FrameReady(cmds)) = self.renderer_channel.try_recv_cmd() {
            self.palette_updated = cmds
                .commands
                .iter()
                .any(|c| matches!(c, GpuCommand::UploadPalette { .. }));
            let mut guard = self.frame_buffer.lock().unwrap();
            std::mem::swap(&mut *guard, &mut self.disp_fb);
            self.renderer_channel.send_ack();
        }
    }

    /// 最後の `swap_frame_buffer()` で `UploadPalette` を受信した場合に true。
    pub fn palette_updated(&self) -> bool {
        self.palette_updated
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
    use nerust_contract_core::GpuCommand;
    use nerust_contract_core::GpuCommandList;
    use nerust_contract_core::channel::frame_channel;
    use nerust_screen_video::{FrameBuffer, PixelFormat};

    fn make_test_video() -> (
        ConsoleVideo,
        nerust_contract_core::channel::FrameChannelConsole,
    ) {
        let mut shared = FrameBuffer::with_capacity(4, 1, PixelFormat::Rgba);
        shared.resize(4, 1);
        let mut disp = FrameBuffer::with_capacity(4, 1, PixelFormat::Rgba);
        disp.resize(4, 1);
        let shared = Arc::new(Mutex::new(shared));
        let (console_ch, renderer_ch) = frame_channel(4);
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
        (
            ConsoleVideo::new(profile, shared, disp, renderer_ch),
            console_ch,
        )
    }

    #[test]
    fn console_video_swap_brings_new_data() {
        let (mut video, console_ch) = make_test_video();
        // Simulate publish_frame: write data to shared, send command
        {
            let mut guard = video.frame_buffer.lock().unwrap();
            guard.as_mut().fill(42);
        }
        console_ch.try_send_frame(GpuCommandList {
            commands: vec![GpuCommand::Blit { slot: 0 }],
        });
        // Before swap: disp_fb is empty (zeros)
        video.with_frame_buffer(|bytes| {
            assert_eq!(bytes[0], 0);
        });
        // After swap: disp_fb has the data
        video.swap_frame_buffer();
        video.with_frame_buffer(|bytes| {
            assert_eq!(bytes[0], 42);
        });
    }
}

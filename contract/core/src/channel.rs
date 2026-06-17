use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, TrySendError};
use std::sync::mpsc::sync_channel;

use crate::GpuCommandList;

/// Emu → Renderer へのコマンド通知
pub enum EmuToRenderer {
    FrameReady(GpuCommandList),
}

/// Console 側のチャネルハンドル
pub struct FrameChannelConsole {
    cmd_tx: SyncSender<EmuToRenderer>,
    ack: Arc<AtomicBool>,
}

impl fmt::Debug for FrameChannelConsole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FrameChannelConsole").finish_non_exhaustive()
    }
}

impl FrameChannelConsole {
    fn new(cmd_tx: SyncSender<EmuToRenderer>, ack: Arc<AtomicBool>) -> Self {
        Self { cmd_tx, ack }
    }

    /// Renderer の ACK を確認し、コマンドを送信する。
    /// Blit を含む場合のみ ACK 確認を行い、ACK 未着またはチャネル Full なら
    /// false（フレームスキップ）。Blit を含まないコマンドは常に送信する。
    pub fn try_send_frame(&self, cmds: GpuCommandList) -> bool {
        let needs_ack = cmds.commands.iter().any(|c| matches!(c, crate::GpuCommand::Blit { .. }));
        if needs_ack && !self.ack.swap(false, Ordering::Acquire) {
            return false;
        }
        match self.cmd_tx.try_send(EmuToRenderer::FrameReady(cmds)) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => false,
        }
    }
}

/// Renderer 側のチャネルハンドル
pub struct FrameChannelRenderer {
    cmd_rx: Receiver<EmuToRenderer>,
    ack: Arc<AtomicBool>,
}

impl fmt::Debug for FrameChannelRenderer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FrameChannelRenderer").finish_non_exhaustive()
    }
}

impl FrameChannelRenderer {
    fn new(cmd_rx: Receiver<EmuToRenderer>, ack: Arc<AtomicBool>) -> Self {
        Self { cmd_rx, ack }
    }

    /// 最新のコマンドを受信する。なければ None。
    pub fn try_recv_cmd(&self) -> Option<EmuToRenderer> {
        match self.cmd_rx.try_recv() {
            Ok(cmd) => Some(cmd),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => None,
        }
    }

    /// フレーム消費を通知する（Console が次のフレームを送信可能になる）。
    pub fn send_ack(&self) {
        self.ack.store(true, Ordering::Release);
    }
}

/// 双方向チャネルを生成する。
/// `(console, renderer)` のペアを返す。
pub fn frame_channel(capacity: usize) -> (FrameChannelConsole, FrameChannelRenderer) {
    let (cmd_tx, cmd_rx) = sync_channel::<EmuToRenderer>(capacity);
    let ack = Arc::new(AtomicBool::new(true)); // 初期状態は「消費済み」
    (
        FrameChannelConsole::new(cmd_tx, ack.clone()),
        FrameChannelRenderer::new(cmd_rx, ack),
    )
}

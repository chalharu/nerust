mod bridge;
mod ui;

use crate::app_menu::UserEvent;

/// 同一プロセス内で settings apply を処理する bridge。
/// 子プロセス pipe ではなく mpsc channel + EventLoopProxy でメインスレッドと通信する。
pub(super) struct ThreadSettingsBridge {
    proxy: EventLoopProxy<UserEvent>,
}

impl ThreadSettingsBridge {
    pub(super) fn new(proxy: EventLoopProxy<UserEvent>) -> Self {
        Self { proxy }
    }
}

impl SettingsBridge for ThreadSettingsBridge {
    fn apply_settings(&mut self, snapshot: &SettingsSnapshot) -> Result<(), String> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.proxy
            .send_event(UserEvent::ApplySettings {
                snapshot: snapshot.clone(),
                reply: reply_tx,
            })
            .map_err(|error| format!("failed to forward settings apply: {error}"))?;
        reply_rx
            .recv()
            .map_err(|error| format!("settings apply response failed: {error}"))?
    }
}
use nerust_gui_runtime::settings::SettingsSnapshot;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use tao::event_loop::EventLoopProxy;

/// Settings の Apply を抽象化するトレイト。
/// 子プロセス版: pipe 経由で親プロセスに送信 + 応答待ち
/// スレッド版: メインスレッドの EventLoopProxy 経由で処理 + 応答待ち
pub(super) trait SettingsBridge: Send {
    fn apply_settings(&mut self, snapshot: &SettingsSnapshot) -> Result<(), String>;
}

// read_message / write_message および ParentToChildMessage / ChildToParentMessage /
// SettingsChildBridge は pipe ベース子プロセス方式で使用していたが、
// スレッドベース方式に移行したため不要になった。
// bridge.rs はテスト用に残している。

/// スレッド内で settings UI (iced) を実行する handle。
/// terminate() で UI スレッドに閉じるよう通知する。
#[derive(Clone)]
pub(crate) struct SettingsHelperHandle {
    close_tx: Arc<Mutex<Option<mpsc::Sender<()>>>>,
}

impl SettingsHelperHandle {
    pub(crate) fn terminate(&self) {
        if let Some(tx) = self.close_tx.lock().unwrap().take() {
            let _ = tx.send(());
        }
    }
}

pub(crate) fn spawn_settings_helper(
    initial_snapshot: SettingsSnapshot,
    proxy: EventLoopProxy<UserEvent>,
) -> Result<SettingsHelperHandle, String> {
    let (close_tx, close_rx) = mpsc::channel::<()>();
    let handle = SettingsHelperHandle {
        close_tx: Arc::new(Mutex::new(Some(close_tx))),
    };

    let bridge: Arc<Mutex<dyn SettingsBridge>> =
        Arc::new(Mutex::new(ThreadSettingsBridge::new(proxy.clone())));
    let bridge_clone = bridge.clone();
    std::thread::Builder::new()
        .name("nerust-tao-settings".into())
        .spawn(move || {
            // iced は独自の event loop を持つため別スレッドで実行。
            // close_rx を受信したら UI を閉じる (iced の window close と同等)。
            // drop(close_rx) でスレッドを終了させることも可。
            let result = ui::run(initial_snapshot, bridge_clone);
            if let Err(error) = result {
                log::warn!("settings helper failed: {error}");
            }
            let _ = proxy.send_event(UserEvent::SettingsClosed);
            // close_rx を明示的にドロップする必要はない (関数終了時に自動ドロップ)
            drop(close_rx);
        })
        .map_err(|error| format!("failed to launch settings helper thread: {error}"))?;

    Ok(handle)
}

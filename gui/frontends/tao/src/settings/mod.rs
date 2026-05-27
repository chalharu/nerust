mod bridge;
mod ui;

use crate::app_menu::UserEvent;
use crate::settings::bridge::{
    ChildToParentMessage, ParentToChildMessage, read_message, write_message,
};
use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_input_schema::SystemId;
use std::io::{BufReader, BufWriter};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use tao::event_loop::EventLoopProxy;

pub const HELPER_FLAG: &str = "settings-ui-helper";

pub fn run_settings_helper_from_stdio() -> Result<(), String> {
    let (snapshot, system_id, bridge) = bridge::SettingsChildBridge::connect_stdio()?;
    ui::run(snapshot, system_id, bridge)
}

#[derive(Clone)]
pub(crate) struct SettingsHelperHandle {
    child: Arc<Mutex<Child>>,
}

impl SettingsHelperHandle {
    pub(crate) fn terminate(&self) {
        let Ok(mut child) = self.child.lock() else {
            return;
        };
        let _ = child.kill();
    }
}

pub(crate) fn spawn_settings_helper(
    initial_snapshot: SettingsSnapshot,
    system_id: SystemId,
    proxy: EventLoopProxy<UserEvent>,
) -> Result<SettingsHelperHandle, String> {
    let current_exe = std::env::current_exe()
        .map_err(|error| format!("failed to resolve current executable: {error}"))?;
    let mut child = Command::new(current_exe)
        .arg(format!("--{HELPER_FLAG}"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|error| format!("failed to spawn settings helper: {error}"))?;
    let child_stdout = child
        .stdout
        .take()
        .ok_or_else(|| "settings helper stdout was unavailable".to_string())?;
    let child_stdin = child
        .stdin
        .take()
        .ok_or_else(|| "settings helper stdin was unavailable".to_string())?;
    let child = Arc::new(Mutex::new(child));
    let worker_child = child.clone();
    std::thread::Builder::new()
        .name("nerust-tao-settings".into())
        .spawn(move || {
            if let Err(error) = bridge_settings_helper(
                worker_child,
                child_stdin,
                child_stdout,
                initial_snapshot,
                system_id,
                proxy.clone(),
            ) {
                log::warn!("settings helper bridge failed: {error}");
                let _ = proxy.send_event(UserEvent::SettingsClosed);
            }
        })
        .map_err(|error| format!("failed to launch settings helper bridge thread: {error}"))?;
    Ok(SettingsHelperHandle { child })
}

fn bridge_settings_helper(
    child: Arc<Mutex<Child>>,
    child_stdin: std::process::ChildStdin,
    child_stdout: std::process::ChildStdout,
    initial_snapshot: SettingsSnapshot,
    system_id: SystemId,
    proxy: EventLoopProxy<UserEvent>,
) -> Result<(), String> {
    let mut reader = BufReader::new(child_stdout);
    let mut writer = BufWriter::new(child_stdin);

    write_message(
        &mut writer,
        &ParentToChildMessage::Init {
            snapshot: initial_snapshot,
            system_id,
        },
    )?;
    while let Some(message) = read_message::<_, ChildToParentMessage>(&mut reader)? {
        match message {
            ChildToParentMessage::Apply(snapshot) => {
                let (reply_tx, reply_rx) = mpsc::channel();
                proxy
                    .send_event(UserEvent::ApplySettings {
                        snapshot,
                        reply: reply_tx,
                    })
                    .map_err(|error| format!("failed to forward settings apply: {error}"))?;
                let result = reply_rx
                    .recv()
                    .map_err(|error| format!("settings apply response failed: {error}"))?;
                write_message(&mut writer, &ParentToChildMessage::ApplyResult(result))?;
            }
        }
    }

    if let Ok(mut child) = child.lock() {
        let _ = child.wait();
    }
    let _ = proxy.send_event(UserEvent::SettingsClosed);
    Ok(())
}

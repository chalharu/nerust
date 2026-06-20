mod bridge;
pub(crate) mod ui;

use crate::app_menu::UserEvent;
use nerust_gui_runtime::settings::SettingsSnapshot;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, Read, Write};
use std::io::{BufReader, BufWriter};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use tao::event_loop::EventLoopProxy;

const MAX_HEADER_BYTES: usize = 16;
const MAX_MESSAGE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Serialize, Deserialize)]
pub(super) enum ParentToChildMessage {
    Init(SettingsSnapshot),
    ApplyResult(Result<(), String>),
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) enum ChildToParentMessage {
    Apply(SettingsSnapshot),
}

pub(super) fn read_message<R: BufRead, T: DeserializeOwned>(
    reader: &mut R,
) -> Result<Option<T>, String> {
    let mut length_line = Vec::new();
    let read = reader
        .take((MAX_HEADER_BYTES + 1) as u64)
        .read_until(b'\n', &mut length_line)
        .map_err(|error| format!("settings bridge read failed: {error}"))?;
    if read == 0 {
        return Ok(None);
    }
    if !length_line.ends_with(b"\n") || length_line.len() > MAX_HEADER_BYTES {
        return Err("settings bridge length header exceeded the supported limit".into());
    }
    let payload_len = std::str::from_utf8(&length_line)
        .map_err(|error| format!("settings bridge length was not UTF-8: {error}"))?
        .trim()
        .parse::<usize>()
        .map_err(|error| format!("settings bridge length parse failed: {error}"))?;
    if payload_len > MAX_MESSAGE_BYTES {
        return Err(format!(
            "settings bridge payload exceeded the supported limit ({payload_len} > {MAX_MESSAGE_BYTES})"
        ));
    }
    let mut payload = vec![0_u8; payload_len];
    reader
        .read_exact(&mut payload)
        .map_err(|error| format!("settings bridge payload read failed: {error}"))?;
    rmp_serde::from_slice(&payload)
        .map(Some)
        .map_err(|error| format!("settings bridge decode failed: {error}"))
}

pub(super) fn write_message<W: Write, T: serde::Serialize>(
    writer: &mut W,
    message: &T,
) -> Result<(), String> {
    let payload = rmp_serde::to_vec_named(message)
        .map_err(|error| format!("settings bridge encode failed: {error}"))?;
    writeln!(writer, "{}", payload.len())
        .map_err(|error| format!("settings bridge length write failed: {error}"))?;
    writer
        .write_all(&payload)
        .map_err(|error| format!("settings bridge payload write failed: {error}"))?;
    writer
        .flush()
        .map_err(|error| format!("settings bridge flush failed: {error}"))
}

pub const HELPER_FLAG: &str = "settings-ui-helper";

pub fn run_settings_helper_from_stdio() -> Result<(), String> {
    let (snapshot, bridge) = bridge::SettingsChildBridge::connect_stdio()?;
    ui::run(snapshot, bridge)
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
    proxy: EventLoopProxy<UserEvent>,
) -> Result<(), String> {
    let mut reader = BufReader::new(child_stdout);
    let mut writer = BufWriter::new(child_stdin);

    write_message(&mut writer, &ParentToChildMessage::Init(initial_snapshot))?;
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

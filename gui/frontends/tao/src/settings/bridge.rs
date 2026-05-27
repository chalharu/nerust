use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_input_schema::SystemId;
use serde::de::DeserializeOwned;
use serde_derive::{Deserialize, Serialize};
use std::io::{self, BufRead, Read, Write};

const MAX_HEADER_BYTES: usize = 16;
const MAX_MESSAGE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Serialize, Deserialize)]
pub(super) enum ParentToChildMessage {
    Init {
        snapshot: SettingsSnapshot,
        system_id: SystemId,
    },
    ApplyResult(Result<(), String>),
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) enum ChildToParentMessage {
    Apply(SettingsSnapshot),
}

pub(super) struct SettingsChildBridge<R, W> {
    reader: R,
    writer: W,
}

impl SettingsChildBridge<io::BufReader<io::Stdin>, io::BufWriter<io::Stdout>> {
    pub(super) fn connect_stdio() -> Result<(SettingsSnapshot, SystemId, Self), String> {
        let stdin = io::BufReader::new(io::stdin());
        let stdout = io::BufWriter::new(io::stdout());
        let mut bridge = Self {
            reader: stdin,
            writer: stdout,
        };
        let Some(ParentToChildMessage::Init {
            snapshot,
            system_id,
        }) = read_message(&mut bridge.reader)?
        else {
            return Err("settings helper did not receive an init message".into());
        };
        Ok((snapshot, system_id, bridge))
    }
}

impl<R: BufRead, W: Write> SettingsChildBridge<R, W> {
    pub(super) fn apply_settings(&mut self, snapshot: &SettingsSnapshot) -> Result<(), String> {
        write_message(
            &mut self.writer,
            &ChildToParentMessage::Apply(snapshot.clone()),
        )?;
        match read_message(&mut self.reader)? {
            Some(ParentToChildMessage::ApplyResult(result)) => result,
            Some(ParentToChildMessage::Init { .. }) => {
                Err("settings helper received an unexpected init message".into())
            }
            None => Err("settings helper lost its parent bridge".into()),
        }
    }
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

#[cfg(test)]
mod tests {
    use super::{
        ChildToParentMessage, ParentToChildMessage, SettingsChildBridge, read_message,
        write_message,
    };
    use nerust_gui_runtime::settings::SettingsSnapshot;
    use nerust_gui_shell::settings::defaults::seed::{
        default_app_state, default_local_settings, default_shared_settings,
    };
    use nerust_input_schema::SystemId;
    use std::io::Cursor;

    fn snapshot() -> SettingsSnapshot {
        SettingsSnapshot {
            shared: default_shared_settings(),
            local: default_local_settings(),
            app_state: default_app_state(),
        }
    }

    #[test]
    fn framed_messages_round_trip() {
        let mut buffer = Vec::new();
        let message = ParentToChildMessage::ApplyResult(Ok(()));

        write_message(&mut buffer, &message).unwrap();

        let decoded = read_message::<_, ParentToChildMessage>(&mut Cursor::new(buffer))
            .unwrap()
            .unwrap();
        assert!(matches!(decoded, ParentToChildMessage::ApplyResult(Ok(()))));
    }

    #[test]
    fn child_bridge_waits_for_parent_apply_result() {
        let snapshot = snapshot();
        let mut parent_inbound = Vec::new();
        write_message(
            &mut parent_inbound,
            &ParentToChildMessage::ApplyResult(Ok(())),
        )
        .unwrap();

        let mut child_outbound = Vec::new();
        let mut bridge = SettingsChildBridge {
            reader: Cursor::new(parent_inbound),
            writer: &mut child_outbound,
        };

        bridge.apply_settings(&snapshot).unwrap();

        let request = read_message::<_, ChildToParentMessage>(&mut Cursor::new(child_outbound))
            .unwrap()
            .unwrap();
        match request {
            ChildToParentMessage::Apply(received) => {
                assert_eq!(received.shared.general, snapshot.shared.general);
            }
        }
    }

    #[test]
    fn init_message_carries_active_system() {
        let snapshot = snapshot();
        let mut buffer = Vec::new();
        write_message(
            &mut buffer,
            &ParentToChildMessage::Init {
                snapshot,
                system_id: SystemId::Snes,
            },
        )
        .unwrap();

        let decoded = read_message::<_, ParentToChildMessage>(&mut Cursor::new(buffer))
            .unwrap()
            .unwrap();

        match decoded {
            ParentToChildMessage::Init { system_id, .. } => assert_eq!(system_id, SystemId::Snes),
            ParentToChildMessage::ApplyResult(_) => panic!("expected init message"),
        }
    }
}

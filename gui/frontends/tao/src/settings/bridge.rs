use super::{ChildToParentMessage, ParentToChildMessage, read_message, write_message};
use nerust_gui_runtime::settings::SettingsSnapshot;
use std::io::{self, BufRead, Write};

pub(super) struct SettingsChildBridge<R, W> {
    reader: R,
    writer: W,
}

impl SettingsChildBridge<io::BufReader<io::Stdin>, io::BufWriter<io::Stdout>> {
    pub(super) fn connect_stdio() -> Result<(SettingsSnapshot, Self), String> {
        let stdin = io::BufReader::new(io::stdin());
        let stdout = io::BufWriter::new(io::stdout());
        let mut bridge = Self {
            reader: stdin,
            writer: stdout,
        };
        let Some(ParentToChildMessage::Init(snapshot)) = read_message(&mut bridge.reader)? else {
            return Err("settings helper did not receive an init message".into());
        };
        Ok((snapshot, bridge))
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
            Some(ParentToChildMessage::Init(_)) => {
                Err("settings helper received an unexpected init message".into())
            }
            None => Err("settings helper lost its parent bridge".into()),
        }
    }
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
}

use std::collections::HashMap;

use super::{ValidationArtifacts, peek_serial};
use crate::{
    error::RomTestError,
    results::{SerialCheck, ValidationOptions},
};

#[derive(Default)]
pub(super) struct SerialArtifacts {
    pub(super) serial: SerialCheckArtifacts,
}

#[derive(Default)]
pub(in crate::runner::validation::artifacts) struct SerialCheckArtifacts {
    pub(in crate::runner::validation::artifacts) checks: Vec<SerialCheck>,
}

/// Expected serial observation for one assertion: the cumulative
/// bytes produced on one channel since power-on (or the last reset),
/// exactly.
pub(in crate::runner::validation) struct ExpectedSerial {
    pub(in crate::runner::validation) frame: u64,
    pub(in crate::runner::validation) channel: String,
    pub(in crate::runner::validation) bytes: Vec<u8>,
}

impl ValidationArtifacts {
    pub(in crate::runner::validation) fn record_serial_assert(
        &mut self,
        case_id: &str,
        serial: &HashMap<String, Vec<u8>>,
        options: ValidationOptions,
        expected: ExpectedSerial,
    ) -> Result<(), RomTestError> {
        let actual = peek_serial(serial, &expected.channel)?;
        if options.check_expectations && actual != expected.bytes.as_slice() {
            self.failures.push(format!(
                "{case_id}: serial mismatch at frame {} channel `{}` (expected {}, actual {})",
                expected.frame,
                expected.channel,
                describe_bytes(&expected.bytes),
                describe_bytes(actual),
            ));
        }
        self.serial.serial.checks.push(SerialCheck {
            frame: expected.frame,
            channel: expected.channel,
            expected_bytes: expected.bytes,
            actual_bytes: actual.to_vec(),
        });
        Ok(())
    }
}

/// Compact description: printable ASCII as text, otherwise hex.
/// Failure lines stay readable for text-emitting test ROMs.
fn describe_bytes(bytes: &[u8]) -> String {
    const PREVIEW: usize = 64;
    let shown = &bytes[..bytes.len().min(PREVIEW)];
    let text = String::from_utf8_lossy(shown);
    let rendered = if text.chars().all(|c| !c.is_control() || c == '\n') {
        format!("{text:?}")
    } else {
        shown
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join("")
    };
    if bytes.len() > PREVIEW {
        format!("{rendered}…({} bytes)", bytes.len())
    } else {
        format!("{rendered}({} bytes)", bytes.len())
    }
}

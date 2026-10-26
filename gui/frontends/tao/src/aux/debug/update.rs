//! Native message adapter. All state transitions live in the common host.

use super::DebugMessage;
use crate::aux::bridge::DebugBridge;

pub(super) fn update_message(bridge: &DebugBridge, message: DebugMessage) {
    bridge.dispatch(message);
}

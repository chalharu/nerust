//! Compatibility imports; debugger policy and storage belong to the shared host.

#[cfg(test)]
pub(crate) use nerust_frontend_host::debugger::lock_recovered;
pub(crate) use nerust_frontend_host::debugger::{
    AppliedDrain, AppliedLive, BridgeLock, DebugBridge, DrainSnapshot,
};

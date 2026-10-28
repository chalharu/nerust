//! Compatibility imports; debugger policy and storage belong to the shared host.

#[cfg(test)]
pub(crate) use nerust_frontend_host::sync::lock_recovered;
pub(crate) use nerust_frontend_viewdefs::debugger::{
    AppliedDrain, AppliedLive, BridgeLock, DebugBridge, DrainSnapshot,
};

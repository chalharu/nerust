//! Compatibility imports; debugger policy and storage belong to the shared host.

pub(crate) use nerust_frontend_host::debugger::{
    BridgeLock, DebugBridge, DrainSnapshot, lock_recovered,
};

//! Shared high-level host. No native event loop, window, or widget dependency.
//! Native adapters translate callbacks; policy and ordering live here.

pub mod aux;
pub mod debug_schedule;
pub mod debugger;
pub mod drain;
pub mod focus;
pub mod frame;
pub mod loop_driver;
pub mod osd;
pub mod session;

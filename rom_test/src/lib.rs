#![allow(
    unused_imports,
    reason = "different harness targets reuse this facade with different subsets of the shared API"
)]

pub mod error;
pub mod events;
pub mod harness;
pub mod manifest;
mod media;
pub mod perf;
pub mod report;
pub mod results;
pub mod runner;
mod serde_helpers;
#[cfg(test)]
mod tests;

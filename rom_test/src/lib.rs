// Copyright (c) 2018 chalharu
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

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

// Copyright (c) 2024 chalharu
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Persistence-side archive and sidecar handling.
//!
//! This crate owns only the outer save-slot archive schema and file-system behavior. `state.bin`
//! stores opaque console state bytes, and the nested core compatibility checks remain owned by the
//! console/core crates during import. Bump `STATE_ARCHIVE_SCHEMA_VERSION` only when archive entry
//! names, metadata fields, or this crate's validation/interpretation rules change.

mod archive;
pub mod error;
mod fs_ops;
mod metadata;
pub mod model;
pub mod sidecar;
pub mod slots;
pub mod thumbnail;
pub mod time;

#[cfg(test)]
mod tests;

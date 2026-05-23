// Copyright (c) 2024 Mitsuharu Seki
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
mod error;
mod fs_ops;
mod metadata;
mod model;
mod sidecar;
mod slots;
mod thumbnail;
mod time;

pub use error::PersistenceError;
pub use model::{LoadedStateSlot, StateSlotSummary};
pub use sidecar::{
    SidecarPaths, load_mapper_save, resolve_sidecars, write_mapper_save, write_recovery_mapper_save,
};
pub use slots::{
    allocate_next_slot_id, delete_state_slot, load_state_slot, scan_state_slots,
    scan_state_slots_for_target, state_slot_path, write_state_slot,
};
pub use thumbnail::ThumbnailSource;
pub use time::{format_slot_saved_at, latest_saved_slot_id};

#[cfg(test)]
mod tests;

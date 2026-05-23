// Copyright (c) 2024 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Core-owned persistence primitives.
//!
//! `PERSISTENCE_SCHEMA_VERSION` is the compatibility boundary for the core crate's
//! serialized mapper-save and machine-state payloads. Bump it whenever either payload,
//! their validation rules, or the meaning of any nested core-owned field changes in a way
//! older bytes should not be accepted. Wrapper formats in other crates must treat the core
//! payload bytes as opaque and manage their own outer schema versions separately.

use thiserror::Error;

pub use crate::RomIdentity;

/// Compatibility version for `MachineStatePayload` and `MapperSavePayload`.
pub(crate) const PERSISTENCE_SCHEMA_VERSION: u32 = 2;

pub(crate) const MAPPER_KIND_NONE: &str = "";
pub(crate) const MAPPER_KIND_ACTION53: &str = "action53";
pub(crate) const MAPPER_KIND_FME7: &str = "fme7";
pub(crate) const MAPPER_KIND_MMC2: &str = "mmc2";
pub(crate) const MAPPER_KIND_MMC3: &str = "mmc3";
pub(crate) const MAPPER_KIND_MMC5: &str = "mmc5";
pub(crate) const MAPPER_KIND_SXROM: &str = "sxrom";

#[derive(Debug, Error)]
pub enum PersistenceError {
    #[error("msgpack decode error: {0}")]
    Decode(#[from] rmp_serde::decode::Error),
    #[error("msgpack encode error: {0}")]
    Encode(#[from] rmp_serde::encode::Error),
    #[error("invalid persistence payload: {0}")]
    Validation(String),
}

#[derive(serde_derive::Serialize, serde_derive::Deserialize)]
pub(crate) struct CartridgeRuntimeState {
    pub mapper_state: crate::mapper_state::MapperState,
    pub extra_kind: String,
    #[serde(with = "serde_bytes")]
    pub extra_body: Vec<u8>,
}

pub(crate) fn encode_payload<T: serde::Serialize>(
    payload: &T,
) -> Result<Vec<u8>, PersistenceError> {
    Ok(rmp_serde::to_vec_named(payload)?)
}

pub(crate) fn decode_payload<T: serde::de::DeserializeOwned>(
    bytes: &[u8],
) -> Result<T, PersistenceError> {
    Ok(rmp_serde::from_slice(bytes)?)
}

pub(crate) fn validate_schema_version(version: u32) -> Result<(), PersistenceError> {
    if version == PERSISTENCE_SCHEMA_VERSION {
        Ok(())
    } else {
        Err(PersistenceError::Validation(format!(
            "unsupported persistence schema version: {version}"
        )))
    }
}

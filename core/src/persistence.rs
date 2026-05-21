// Copyright (c) 2024 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use crate::{MirrorMode, RomFormat};
use thiserror::Error;

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

#[derive(serde_derive::Serialize, serde_derive::Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub struct RomIdentity {
    pub format: RomFormat,
    pub mapper_type: u16,
    pub sub_mapper_type: u8,
    pub mirror_mode: MirrorMode,
    pub has_battery: bool,
    pub trainer_len: usize,
    pub prg_rom_len: usize,
    pub chr_rom_len: usize,
    pub prg_ram_len: usize,
    pub save_prg_ram_len: usize,
    pub chr_ram_len: usize,
    pub save_chr_ram_len: usize,
    pub prg_rom_crc64: u64,
    pub chr_rom_crc64: u64,
    pub trainer_crc64: u64,
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

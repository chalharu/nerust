// Copyright (c) 2024 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use crate::persistence_error::PersistenceError;

/// Compatibility version for `MachineStatePayload` and `MapperSavePayload`.
pub(crate) const PERSISTENCE_SCHEMA_VERSION: u32 = 2;

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

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
        Err(PersistenceError::UnsupportedVersion(version))
    }
}

/// Version accessor so the envelope parses the body once: success validates
/// the body's own version; only failures fall back to a minimal header parse
/// to keep unsupported-version errors outranking corrupt bodies.
pub(crate) trait HasSchemaVersion {
    fn schema_version(&self) -> u32;
}

#[derive(serde::Deserialize)]
struct VersionHeader {
    schema_version: u32,
}

pub(crate) fn decode_versioned_payload<T>(bytes: &[u8]) -> Result<T, PersistenceError>
where
    T: serde::de::DeserializeOwned + HasSchemaVersion,
{
    match decode_payload::<T>(bytes) {
        Ok(body) => {
            validate_schema_version(body.schema_version())?;
            Ok(body)
        }
        Err(body_error) => {
            if let Ok(header) = decode_payload::<VersionHeader>(bytes) {
                validate_schema_version(header.schema_version)?;
            }
            Err(body_error)
        }
    }
}

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

fn require_supported<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let version = <u32 as serde::Deserialize>::deserialize(deserializer)?;
    if version == PERSISTENCE_SCHEMA_VERSION {
        Ok(version)
    } else {
        Err(serde::de::Error::custom("unsupported schema version"))
    }
}

/// Single-parse envelope: supported versions land in `Supported` with the body;
/// anything else lands in `Other` carrying only its version for the precedence error.
#[derive(serde::Deserialize)]
#[serde(untagged)]
enum Versioned<T> {
    Supported {
        #[serde(deserialize_with = "require_supported")]
        schema_version: u32,
        #[serde(flatten)]
        body: T,
    },
    Other {
        schema_version: u32,
    },
}

pub(crate) fn decode_versioned_payload<T: serde::de::DeserializeOwned>(
    bytes: &[u8],
) -> Result<T, PersistenceError> {
    match decode_payload::<Versioned<T>>(bytes)? {
        Versioned::Supported { body, .. } => Ok(body),
        Versioned::Other { schema_version } => {
            validate_schema_version(schema_version)?;
            // Supported version but undecodable body: report the body fault.
            Ok(decode_payload::<T>(bytes)?)
        }
    }
}

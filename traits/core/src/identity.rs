use std::{
    collections::HashSet,
    fmt,
    sync::{Mutex, OnceLock},
};

use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{self, Unexpected, Visitor},
};

/// システム識別子。CoreFactory impl のみが生成する。
/// 比較は `Eq` 経由のみ。生文字列の取り出しは不可。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SystemId(&'static str);

impl SystemId {
    pub const fn new(id: &'static str) -> Self {
        Self(id)
    }
}

impl fmt::Display for SystemId {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl Serialize for SystemId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.0)
    }
}

impl<'de> Deserialize<'de> for SystemId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_str(SystemIdVisitor)
    }
}

struct SystemIdVisitor;

fn intern_system_id(value: &str) -> &'static str {
    static INTERNED: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();

    let mut interned = INTERNED
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(existing) = interned.get(value) {
        return existing;
    }

    let value = Box::leak(value.to_owned().into_boxed_str());
    interned.insert(value);
    value
}

fn is_valid_system_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_' | b'.')
        })
}

impl<'de> Visitor<'de> for SystemIdVisitor {
    type Value = SystemId;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a system identifier string")
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<SystemId, E> {
        let normalized = match v {
            "Nes" | "nes" => "nes",
            "Snes" | "snes" => "snes",
            "Ps1" | "ps1" => "ps1",
            "MegaDrive" | "megadrive" => "megadrive",
            other if is_valid_system_id(other) => intern_system_id(other),
            other => return Err(E::invalid_value(Unexpected::Str(other), &self)),
        };
        Ok(SystemId(normalized))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemIdentity {
    pub system_id: SystemId,
    pub identity_bytes: Vec<u8>,
}

impl SystemIdentity {
    pub fn new(system_id: SystemId, identity_bytes: Vec<u8>) -> Self {
        Self {
            system_id,
            identity_bytes,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SystemId;

    #[test]
    fn arbitrary_valid_id_round_trips_through_serde() {
        let id = SystemId::new("game-boy.color");
        let encoded = serde_json::to_string(&id).unwrap();
        let decoded: SystemId = serde_json::from_str(&encoded).unwrap();

        assert_eq!(decoded, id);
    }

    #[test]
    fn invalid_id_is_rejected() {
        assert!(serde_json::from_str::<SystemId>("\"Game Boy\"").is_err());
    }
}

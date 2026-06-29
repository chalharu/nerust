use std::fmt;

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

impl<'de> Visitor<'de> for SystemIdVisitor {
    type Value = SystemId;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a system identifier string")
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<SystemId, E> {
        Ok(SystemId(match v {
            "Nes" | "nes" => "nes",
            "Snes" | "snes" => "snes",
            "Ps1" | "ps1" => "ps1",
            "MegaDrive" | "megadrive" => "megadrive",
            other => return Err(E::invalid_value(Unexpected::Str(other), &self)),
        }))
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

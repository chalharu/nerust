use serde::de::{self, Visitor};
use std::fmt;

pub(super) fn parse_hex_u64(value: &str) -> Result<u64, String> {
    let trimmed = value.trim().replace('_', "");
    let digits = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"));
    if let Some(digits) = digits {
        u64::from_str_radix(digits, 16)
            .map_err(|error| format!("invalid hexadecimal value `{value}`: {error}"))
    } else {
        trimmed
            .parse::<u64>()
            .map_err(|error| format!("invalid integer value `{value}`: {error}"))
    }
}

pub(super) fn parse_hex_u16(value: &str) -> Result<u16, String> {
    let parsed = parse_hex_u64(value)?;
    u16::try_from(parsed).map_err(|_| format!("value `{value}` does not fit in u16"))
}

pub(super) fn parse_hex_u8(value: &str) -> Result<u8, String> {
    let parsed = parse_hex_u64(value)?;
    u8::try_from(parsed).map_err(|_| format!("value `{value}` does not fit in u8"))
}

pub(super) mod hex_u8 {
    use super::*;
    use serde::{Deserializer, Serializer};

    pub fn serialize<S>(value: &u8, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&format!("0x{value:02X}"))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<u8, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(HexValueVisitor)
    }

    struct HexValueVisitor;

    impl<'de> Visitor<'de> for HexValueVisitor {
        type Value = u8;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a hexadecimal string like 0x12 or an unsigned 8-bit integer")
        }

        fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            u8::try_from(value)
                .map_err(|_| E::custom(format!("value `{value}` does not fit in u8")))
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            parse_hex_u8(value).map_err(E::custom)
        }

        fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            parse_hex_u8(&value).map_err(E::custom)
        }
    }
}

pub(super) mod hex_u16 {
    use super::*;
    use serde::{Deserializer, Serializer};

    pub fn serialize<S>(value: &u16, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&format!("0x{value:04X}"))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<u16, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(HexValueVisitor)
    }

    struct HexValueVisitor;

    impl<'de> Visitor<'de> for HexValueVisitor {
        type Value = u16;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a hexadecimal string like 0x1234 or an unsigned 16-bit integer")
        }

        fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            u16::try_from(value)
                .map_err(|_| E::custom(format!("value `{value}` does not fit in u16")))
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            parse_hex_u16(value).map_err(E::custom)
        }

        fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            parse_hex_u16(&value).map_err(E::custom)
        }
    }
}

pub(super) mod hex_u64 {
    use super::*;
    use serde::{Deserializer, Serializer};

    pub fn serialize<S>(value: &u64, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&format!("0x{value:016X}"))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<u64, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(HexValueVisitor)
    }

    struct HexValueVisitor;

    impl<'de> Visitor<'de> for HexValueVisitor {
        type Value = u64;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a hexadecimal string like 0x0123 or an unsigned integer")
        }

        fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(value)
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            parse_hex_u64(value).map_err(E::custom)
        }

        fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            parse_hex_u64(&value).map_err(E::custom)
        }
    }
}

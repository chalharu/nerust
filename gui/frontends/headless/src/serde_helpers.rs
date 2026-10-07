use std::fmt;

use serde::de::{self, Visitor};

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

pub(super) fn parse_hex_u32(value: &str) -> Result<u32, String> {
    let parsed = parse_hex_u64(value)?;
    u32::try_from(parsed).map_err(|_| format!("value `{value}` does not fit in u32"))
}

pub(super) fn parse_hex_u8(value: &str) -> Result<u8, String> {
    let parsed = parse_hex_u64(value)?;
    u8::try_from(parsed).map_err(|_| format!("value `{value}` does not fit in u8"))
}

pub(super) mod hex_u8 {
    use serde::{Deserializer, Serializer};

    use super::*;

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

pub(super) mod hex_u32 {
    use serde::{Deserializer, Serializer};

    use super::*;

    pub fn serialize<S>(value: &u32, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&format!("0x{value:08X}"))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<u32, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(HexValueVisitor)
    }

    struct HexValueVisitor;

    impl<'de> Visitor<'de> for HexValueVisitor {
        type Value = u32;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter
                .write_str("a hexadecimal string like 0x12345678 or an unsigned 32-bit integer")
        }

        fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            u32::try_from(value)
                .map_err(|_| E::custom(format!("value `{value}` does not fit in u32")))
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            parse_hex_u32(value).map_err(E::custom)
        }

        fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            parse_hex_u32(&value).map_err(E::custom)
        }
    }
}

pub(super) mod hex_u64 {
    use serde::{Deserializer, Serializer};

    use super::*;

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

/// Register expectation map (`name: value`) with hex-tolerant values.
/// Keys are system register names (e.g. `a`, `pc`); unknown names fail
/// loudly at read time, never silently.
pub(super) mod hex_u64_map {
    use std::collections::BTreeMap;

    use serde::{Deserializer, Serializer};

    use super::{Visitor, de, fmt, parse_hex_u64};

    pub fn serialize<S>(value: &BTreeMap<String, u64>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(value.len()))?;
        for (key, val) in value {
            map.serialize_entry(key, &format!("0x{val:X}"))?;
        }
        map.end()
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<BTreeMap<String, u64>, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(HexMapVisitor)
    }

    struct HexMapVisitor;

    impl<'de> Visitor<'de> for HexMapVisitor {
        type Value = BTreeMap<String, u64>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a map of register names to hex strings or integers")
        }

        fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
        where
            A: de::MapAccess<'de>,
        {
            let mut out = BTreeMap::new();
            while let Some(key) = map.next_key::<String>()? {
                let value = map.next_value::<HexU64>()?;
                out.insert(key, value.0);
            }
            Ok(out)
        }
    }

    struct HexU64(u64);

    impl<'de> serde::Deserialize<'de> for HexU64 {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            deserializer.deserialize_any(HexU64Visitor)
        }
    }

    struct HexU64Visitor;

    impl Visitor<'_> for HexU64Visitor {
        type Value = HexU64;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a hexadecimal string like 0x12 or an unsigned integer")
        }

        fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(HexU64(value))
        }

        fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            u64::try_from(value)
                .map(HexU64)
                .map_err(|_| E::custom(format!("value `{value}` does not fit in u64")))
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            parse_hex_u64(value).map(HexU64).map_err(E::custom)
        }

        fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            parse_hex_u64(&value).map(HexU64).map_err(E::custom)
        }
    }
}

/// Serial expectation bytes as one `0x`-prefixed hex string
/// (`bytes: "0x506173736564"`). Test ROMs emit text over the link, so
/// exact bytes stay debuggable; hashes would hide the verdict.
pub(super) mod hex_bytes {
    use serde::{Deserializer, Serializer};

    use super::{Visitor, de, fmt, parse_hex_bytes};

    pub fn serialize<S>(value: &Vec<u8>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut text = String::with_capacity(2 + value.len() * 2);
        text.push_str("0x");
        for byte in value {
            text.push_str(&format!("{byte:02X}"));
        }
        serializer.serialize_str(&text)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_str(HexBytesVisitor)
    }

    struct HexBytesVisitor;

    impl<'de> Visitor<'de> for HexBytesVisitor {
        type Value = Vec<u8>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a hexadecimal string like 0x50617373")
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            parse_hex_bytes(value).map_err(E::custom)
        }

        fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            parse_hex_bytes(&value).map_err(E::custom)
        }
    }
}

pub(super) fn parse_hex_bytes(value: &str) -> Result<Vec<u8>, String> {
    let trimmed = value.trim().replace('_', "");
    let digits = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
        .ok_or_else(|| format!("invalid hexadecimal bytes `{value}`: missing 0x prefix"))?;
    if digits.len() % 2 != 0 {
        return Err(format!(
            "invalid hexadecimal bytes `{value}`: odd digit count"
        ));
    }
    (0..digits.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&digits[i..i + 2], 16)
                .map_err(|error| format!("invalid hexadecimal bytes `{value}`: {error}"))
        })
        .collect()
}

/// Fixed-size header prepended to core state bytes for self-identification.
///
/// # Format
///
/// ```text
/// offset  size  field
///      0     4  magic: b"NRST"
///      4     4  version: u32 LE (schema version, starts at 1)
///      8     4  data_size: u32 LE (size of core state bytes following the header)
/// ```
///
/// Total: 12 bytes. The header is followed by `data_size` bytes of core state.
/// Wrapping/unwrapping is done at the `Console` layer — Core is not modified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveStateHeader {
    pub version: u32,
    pub data_size: u32,
}

const SAVE_STATE_MAGIC: [u8; 4] = *b"NRST";
const SAVE_STATE_HEADER_SIZE: usize = 12;

impl SaveStateHeader {
    pub fn new(core_state: &[u8]) -> Self {
        Self {
            version: 1,
            data_size: core_state.len() as u32,
        }
    }

    pub fn encode(&self) -> [u8; SAVE_STATE_HEADER_SIZE] {
        let mut buf = [0u8; SAVE_STATE_HEADER_SIZE];
        buf[0..4].copy_from_slice(&SAVE_STATE_MAGIC);
        buf[4..8].copy_from_slice(&self.version.to_le_bytes());
        buf[8..12].copy_from_slice(&self.data_size.to_le_bytes());
        buf
    }

    pub fn decode(buf: &[u8; SAVE_STATE_HEADER_SIZE]) -> Result<Self, &'static str> {
        if buf[0..4] != SAVE_STATE_MAGIC {
            return Err("invalid save state magic");
        }
        let version = u32::from_le_bytes(buf[4..8].try_into().unwrap());
        let data_size = u32::from_le_bytes(buf[8..12].try_into().unwrap());
        Ok(Self { version, data_size })
    }
}

/// Wraps core state bytes with a `SaveStateHeader`. Allocates a new `Vec`
/// with the header prepended. This is a cold path (save/load, not per-frame).
pub fn save_state_with_header(core_state: Vec<u8>) -> Vec<u8> {
    let header = SaveStateHeader::new(&core_state);
    let mut buf = Vec::with_capacity(SAVE_STATE_HEADER_SIZE + core_state.len());
    buf.extend_from_slice(&header.encode());
    buf.extend_from_slice(&core_state);
    buf
}

/// Unwraps a `SaveStateHeader`-prefixed byte slice. Returns the inner core
/// state bytes on success, or an error description on failure.
pub fn load_state_from_header(data: &[u8]) -> Result<&[u8], &'static str> {
    if data.len() < SAVE_STATE_HEADER_SIZE {
        return Err("save state data too short");
    }
    let header_buf: &[u8; SAVE_STATE_HEADER_SIZE] = data[..SAVE_STATE_HEADER_SIZE]
        .try_into()
        .map_err(|_| "failed to parse save state header")?;
    let header = SaveStateHeader::decode(header_buf)?;
    let data_size = header.data_size as usize;
    let core_state = &data[SAVE_STATE_HEADER_SIZE..];
    if core_state.len() < data_size {
        return Err("save state data truncated");
    }
    Ok(&core_state[..data_size])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_version() {
        let core_state = vec![1, 2, 3, 4, 5];
        let wrapped = save_state_with_header(core_state.clone());
        let inner = load_state_from_header(&wrapped).unwrap();
        assert_eq!(inner, &[1, 2, 3, 4, 5]);
    }

    #[test]
    fn rejects_truncated_data() {
        let core_state = vec![42; 100];
        let wrapped = save_state_with_header(core_state);
        let truncated = &wrapped[..wrapped.len() - 10];
        assert!(load_state_from_header(truncated).is_err());
    }

    #[test]
    fn rejects_bad_magic() {
        let bad: [u8; 12] = [0; 12];
        assert!(load_state_from_header(&bad).is_err());
    }
}

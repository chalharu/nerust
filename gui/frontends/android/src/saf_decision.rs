//! SAF transport decisions as pure predicates.
//! Host-compiled so the capability ladder and error taxonomy are unit-tested.

/// Typed SAF failure: only a version skew may fall back.
/// Missing documents and I/O faults are hard errors on the chosen path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SafError {
    Missing,
    VersionSkew,
    Io(String),
}

impl SafError {
    /// Fallback is reserved for a bridge too old for the chosen path.
    pub(crate) fn fallback_eligible(&self) -> bool {
        matches!(self, SafError::VersionSkew)
    }

    /// Missing-document probe result shared by every path.
    pub(crate) fn missing() -> Self {
        SafError::Missing
    }

    /// Old-APK signal: the method the path needs does not exist.
    pub(crate) fn version_skew() -> Self {
        SafError::VersionSkew
    }

    /// Transport fault: surfaced, never retried on another path.
    pub(crate) fn io(message: impl ToString) -> Self {
        SafError::Io(message.to_string())
    }
}

/// One-shot capability probe: newest path the APK offers.
/// Probed once per backend; writes chunk whenever reads stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SafCapability {
    Streamed,
    ChunkedBytes,
    ChunkedBase64,
    Legacy,
}

/// Write-side path for a probed capability: reads stream, writes chunk.
impl SafCapability {
    pub(crate) fn write_capability(self) -> Self {
        match self {
            SafCapability::Streamed => SafCapability::ChunkedBytes,
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_version_skew_falls_back() {
        assert!(!SafError::missing().fallback_eligible());
        assert!(SafError::version_skew().fallback_eligible());
        assert!(!SafError::io("disk gone").fallback_eligible());
    }

    #[test]
    fn stream_capability_writes_through_chunks() {
        assert_eq!(
            SafCapability::Streamed.write_capability(),
            SafCapability::ChunkedBytes
        );
        assert_eq!(
            SafCapability::ChunkedBase64.write_capability(),
            SafCapability::ChunkedBase64
        );
        assert_eq!(
            SafCapability::Legacy.write_capability(),
            SafCapability::Legacy
        );
    }
}

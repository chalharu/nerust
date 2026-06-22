use crate::identity::SystemIdentity;

/// Compatibility guard for save-state loading.
///
/// Both `identity` and `options_bytes` are opaque blobs whose
/// interpretation is system-specific (defined by the core implementation).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateCompatibility {
    pub identity: SystemIdentity,
    /// Opaque; interpreted by the CoreFactory / system core.
    pub options_bytes: Vec<u8>,
}

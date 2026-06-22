use crate::identity::SystemIdentity;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateCompatibility {
    pub identity: SystemIdentity,
    pub options_bytes: Vec<u8>,
}

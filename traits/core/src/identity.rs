use nerust_input_traits::SystemId;

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

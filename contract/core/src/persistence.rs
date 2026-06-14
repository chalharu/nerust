use crate::{options::CoreOptions, rom::RomIdentity};
use nerust_input_schema::SystemId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanonicalMediaIdentity {
    Rom(RomIdentity),
}

impl CanonicalMediaIdentity {
    pub const fn rom(rom_identity: RomIdentity) -> Self {
        Self::Rom(rom_identity)
    }

    pub const fn rom_identity(self) -> RomIdentity {
        match self {
            Self::Rom(rom_identity) => rom_identity,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PersistenceIdentity {
    pub system_id: SystemId,
    pub media: CanonicalMediaIdentity,
}

impl PersistenceIdentity {
    pub const fn rom(system_id: SystemId, rom_identity: RomIdentity) -> Self {
        Self {
            system_id,
            media: CanonicalMediaIdentity::Rom(rom_identity),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateCompatibility {
    pub rom_identity: RomIdentity,
    pub options: CoreOptions,
}

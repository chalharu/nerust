#![allow(
    unused_imports,
    reason = "localized re-export facade intentionally exposes shared MMC3-family types"
)]

pub(crate) use crate::CartridgeData;
pub(crate) use crate::CartridgeDataParts;
pub(crate) use crate::Mmc3IrqVariant;
pub(crate) use crate::RomFormat;
pub(crate) use crate::cartridge_error::CartridgeError;
pub(crate) use crate::mapper::{CartridgeDataDao, Mapper};
pub(crate) use crate::mapper_state::{MapperState, MapperStateDao, MappingMode};
pub(crate) use crate::status::mirror_mode::MirrorMode;

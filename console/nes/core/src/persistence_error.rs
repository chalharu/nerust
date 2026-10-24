// Copyright (c) 2024 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum PersistenceError {
    #[error("msgpack decode error: {0}")]
    Decode(#[from] rmp_serde::decode::Error),
    #[error("msgpack encode error: {0}")]
    Encode(#[from] rmp_serde::encode::Error),
    #[error("invalid persistence payload: unsupported persistence schema version: {0}")]
    UnsupportedVersion(u32),
    #[error("invalid persistence payload: ROM identity mismatch")]
    RomIdentityMismatch,
    #[error("invalid persistence payload: runtime options mismatch")]
    OptionsMismatch,
    #[error("invalid persistence payload: unexpected mapper-specific state for this mapper")]
    UnexpectedMapperState,
    #[error("invalid persistence payload: unexpected {0} runtime kind")]
    MapperRuntimeKind(&'static str),
    #[error("invalid persistence payload: MMC5 EXRAM length mismatch")]
    Mmc5ExramLength,
    #[error("invalid persistence payload: cartridge does not expose persistent mapper save memory")]
    NoPersistentMemory,
    #[error("invalid persistence payload: persistent mapper memory length mismatch")]
    PersistentMemoryLength,
    #[error(
        "invalid persistence payload: persistent mapper memory exceeds available backing store"
    )]
    PersistentMemoryBounds,
    #[error("invalid persistence payload: mapper backing store length mismatch")]
    MapperBackingLength,
    #[error("invalid persistence payload: mapper battery configuration mismatch")]
    MapperBatteryConfiguration,
    #[error("invalid persistence payload: mapper character mapping mode mismatch")]
    MapperCharacterMapping,
    #[error("invalid persistence payload: {table} page table entry {index} out of bounds")]
    PageTableBounds { table: &'static str, index: usize },
    #[error("invalid persistence payload: CPU opcode overflow")]
    CpuOpcode,
    #[error("invalid persistence payload: CPU address overflow")]
    CpuAddress,
    #[error("invalid persistence payload: CPU temporary address overflow")]
    CpuTemporaryAddress,
    #[error("invalid persistence payload: CPU step overflow")]
    CpuStep,
    #[error("invalid persistence payload: PPU sprite index overflow")]
    PpuSpriteIndex,
    #[error(
        "invalid persistence payload: PPU sprite index terminal state is only valid after the final sprite fetch"
    )]
    PpuTerminalSpriteIndex,
    #[error("invalid persistence payload: PPU sprite count overflow")]
    PpuSpriteCount,
    #[error("invalid persistence payload: PPU secondary OAM address overflow")]
    PpuSecondaryOamAddress,
    #[error("invalid persistence payload: APU pulse duty mode overflow")]
    ApuPulseDutyMode,
    #[error("invalid persistence payload: APU pulse duty value overflow")]
    ApuPulseDutyValue,
    #[error("invalid persistence payload: MMC5 pulse duty mode overflow")]
    Mmc5PulseDutyMode,
    #[error("invalid persistence payload: MMC5 pulse duty value overflow")]
    Mmc5PulseDutyValue,
}

impl From<PersistenceError> for nerust_core_traits::CoreError {
    fn from(error: PersistenceError) -> Self {
        use nerust_core_traits::CoreFault;
        let kind = match &error {
            PersistenceError::UnsupportedVersion(_)
            | PersistenceError::RomIdentityMismatch
            | PersistenceError::OptionsMismatch => CoreFault::Incompatible,
            _ => CoreFault::Corrupt,
        };
        Self::Persistence {
            kind,
            source: Box::new(error),
        }
    }
}

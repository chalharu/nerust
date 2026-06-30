use super::{
    Cartridge,
    shared::{
        IrqVariant, LegacyIrqState, LegacyMapper4State, Mapper4Config, Mapper4Shared,
        Mapper4Wrapper, PrgRamModel,
    },
};
use crate::{
    cartridge_rom::CartridgeData, cartridge_runtime_state::CartridgeRuntimeState,
    mapper_state::MapperState, persistence_error::PersistenceError,
};

#[derive(serde::Serialize)]
pub(super) struct Mmc3 {
    pub(super) shared: Mapper4Shared,
}

#[derive(serde::Deserialize)]
#[serde(untagged)]
enum Mmc3Deserialized {
    Current { shared: Mapper4Shared },
    Legacy(LegacyMmc3State),
}

#[derive(serde::Deserialize)]
struct LegacyMmc3State {
    cartridge_data: CartridgeData,
    state: MapperState,
    bank_select: u8,
    bank_data: [u8; 8],
    mirroring: u8,
    program_ram_protect: u8,
    irq: LegacyIrqUnit,
    prg_ram_model: LegacyPrgRamModel,
}

#[derive(serde::Deserialize)]
enum LegacyIrqVariant {
    Sharp,
    NecOldStyle,
}

#[derive(serde::Deserialize)]
struct LegacyIrqUnit {
    variant: LegacyIrqVariant,
    latch: u8,
    reload: bool,
    counter: u8,
    enabled: bool,
    last_a12_high: bool,
    last_a12_low_tick: u64,
}

#[derive(serde::Deserialize)]
enum LegacyPrgRamModel {
    Standard,
    Mmc6,
}

impl Mmc3 {
    pub(super) fn new(data: CartridgeData, bus_conflicts: bool) -> Self {
        Self {
            shared: Mapper4Shared::new(data, Mapper4Config::mmc3(bus_conflicts)),
        }
    }

    fn from_deserialized(deserialized: Mmc3Deserialized) -> Self {
        match deserialized {
            Mmc3Deserialized::Current { shared } => Self { shared },
            Mmc3Deserialized::Legacy(legacy) => Self {
                shared: Mapper4Shared::from_legacy_state(
                    legacy.cartridge_data,
                    LegacyMapper4State {
                        state: legacy.state,
                        bank_select: legacy.bank_select,
                        bank_data: legacy.bank_data,
                        mirroring: legacy.mirroring,
                        program_ram_protect: legacy.program_ram_protect,
                        irq: LegacyIrqState {
                            variant: legacy.irq.variant.into(),
                            latch: legacy.irq.latch,
                            reload: legacy.irq.reload,
                            counter: legacy.irq.counter,
                            enabled: legacy.irq.enabled,
                            last_a12_high: legacy.irq.last_a12_high,
                            last_a12_low_tick: legacy.irq.last_a12_low_tick,
                        },
                        prg_ram_model: legacy.prg_ram_model.into(),
                    },
                ),
            },
        }
    }
}

impl From<LegacyIrqVariant> for IrqVariant {
    fn from(value: LegacyIrqVariant) -> Self {
        match value {
            LegacyIrqVariant::Sharp => Self::Sharp,
            LegacyIrqVariant::NecOldStyle => Self::NecOldStyle,
        }
    }
}

impl From<LegacyPrgRamModel> for PrgRamModel {
    fn from(value: LegacyPrgRamModel) -> Self {
        match value {
            LegacyPrgRamModel::Standard => Self::Standard,
            LegacyPrgRamModel::Mmc6 => Self::Mmc6,
        }
    }
}

impl<'de> serde::Deserialize<'de> for Mmc3 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Ok(Self::from_deserialized(
            <Mmc3Deserialized as serde::Deserialize>::deserialize(deserializer)?,
        ))
    }
}

#[typetag::serde]
impl Cartridge for Mmc3 {
    fn export_runtime_state(&self) -> Result<CartridgeRuntimeState, PersistenceError> {
        self.shared.export_runtime_state()
    }

    fn import_runtime_state(
        &mut self,
        state: CartridgeRuntimeState,
    ) -> Result<(), PersistenceError> {
        self.shared.import_runtime_state(state)
    }
}

impl Mapper4Wrapper for Mmc3 {
    const NAME: &'static str = "MMC3 (Mapper4)";

    fn shared_ref(&self) -> &Mapper4Shared {
        &self.shared
    }

    fn shared_mut(&mut self) -> &mut Mapper4Shared {
        &mut self.shared
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CartridgeData, IrqVariant, LegacyIrqUnit, LegacyIrqVariant, LegacyMmc3State,
        LegacyPrgRamModel, MapperState, Mmc3, Mmc3Deserialized, PrgRamModel,
    };
    use crate::{
        cartridge_data_parts::CartridgeDataParts, mapper::Mapper, mirror::MirrorMode,
        rom_format::RomFormat,
    };

    fn test_data(sub_mapper_type: u8) -> CartridgeData {
        CartridgeData::new(CartridgeDataParts {
            format: RomFormat::Nes20,
            prog_rom: vec![0; 0x8000],
            char_rom: vec![0; 0x2000],
            pram_length: 0,
            save_pram_length: 0,
            vram_length: 0,
            save_vram_length: 0,
            mapper_type: 4,
            mirror_mode: MirrorMode::Horizontal,
            has_battery: false,
            sub_mapper_type,
            trainer: Vec::new(),
        })
        .expect("test cartridge data should be valid")
    }

    #[test]
    fn legacy_flat_payload_deserializes_into_shared_wrapper() {
        let mapper = Mmc3::from_deserialized(Mmc3Deserialized::Legacy(LegacyMmc3State {
            cartridge_data: test_data(1),
            state: MapperState::new(),
            bank_select: 0x20,
            bank_data: [0, 0, 0, 0, 0, 0, 0, 1],
            mirroring: 0,
            program_ram_protect: 0xF0,
            irq: LegacyIrqUnit {
                variant: LegacyIrqVariant::NecOldStyle,
                latch: 3,
                reload: true,
                counter: 2,
                enabled: true,
                last_a12_high: false,
                last_a12_low_tick: 9,
            },
            prg_ram_model: LegacyPrgRamModel::Mmc6,
        }));

        assert_eq!(mapper.shared.prg_ram_model(), PrgRamModel::Mmc6);
        assert_eq!(mapper.shared.irq_variant(), IrqVariant::NecOldStyle);
        assert!(!Mapper::bus_conflicts(&mapper.shared));
    }
}

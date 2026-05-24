use crate::OpenBusReadResult;
use crate::apu::Core as Apu;
use crate::cart_device::Cartridge;
use crate::cartridge;
use crate::cartridge_data::CartridgeData;
use crate::controller::Controller;
use crate::cpu::Core as Cpu;
use crate::persistence;
use crate::ppu::Core as Ppu;
use crc::{CRC_64_XZ, Crc, Digest};
use nerust_contract::{CoreOptions, MirrorMode, RomFormat, RomIdentity};
use nerust_screen_traits::Screen;
use nerust_sound_traits::MixerInput;

const CRC64_LEGACY_ECMA: Crc<u64> = Crc::<u64>::new(&CRC_64_XZ);

struct Crc64Hasher(Digest<'static, u64>);

impl Crc64Hasher {
    fn new() -> Self {
        Self(CRC64_LEGACY_ECMA.digest())
    }
}

fn crc64(bytes: &[u8]) -> u64 {
    let mut hasher = Crc64Hasher::new();
    hasher.0.update(bytes);
    hasher.0.finalize()
}

pub type Error = Box<dyn std::error::Error + Send + Sync + 'static>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RomInfo {
    pub format: RomFormat,
    pub mapper_type: u16,
    pub sub_mapper_type: u8,
    pub mirror_mode: MirrorMode,
    pub has_battery: bool,
    pub trainer_len: usize,
    pub prg_rom_len: usize,
    pub chr_rom_len: usize,
    pub prg_ram_len: usize,
    pub save_prg_ram_len: usize,
    pub chr_ram_len: usize,
    pub save_chr_ram_len: usize,
    pub raw_file_len: usize,
    pub body_len: usize,
}

#[derive(serde_derive::Serialize, serde_derive::Deserialize)]
pub struct Core {
    cpu: Cpu,
    ppu: Ppu,
    apu: Apu,
    cartridge: Box<dyn Cartridge>,
    options: CoreOptions,
}

/// Core-owned mapper-save payload.
///
/// This payload is intentionally scoped to battery-backed mapper RAM/VRAM plus the ROM identity
/// needed to reject incompatible imports. The `options` field is recorded for diagnostics and
/// fixture visibility today, but mapper-save import compatibility is currently enforced only by
/// `rom_identity`; changing that policy would require an explicit compatibility decision.
#[derive(serde_derive::Serialize, serde_derive::Deserialize)]
pub(crate) struct MapperSavePayload {
    pub(crate) schema_version: u32,
    pub(crate) rom_identity: RomIdentity,
    pub(crate) options: CoreOptions,
    #[serde(with = "serde_bytes")]
    pub(crate) prg_ram: Vec<u8>,
    #[serde(with = "serde_bytes")]
    pub(crate) chr_ram: Vec<u8>,
}

/// Core-owned full machine-state payload.
///
/// This schema owns CPU/PPU/APU/cartridge runtime bytes and the import validation that protects
/// them. Both `rom_identity` and `options` are part of the compatibility contract for imports, so
/// any incompatible change here requires a `PERSISTENCE_SCHEMA_VERSION` bump.
#[derive(serde_derive::Serialize, serde_derive::Deserialize)]
pub(crate) struct MachineStatePayload {
    pub(crate) schema_version: u32,
    pub(crate) rom_identity: RomIdentity,
    pub(crate) options: CoreOptions,
    pub(crate) cpu: Cpu,
    pub(crate) ppu: Ppu,
    pub(crate) apu: Apu,
    pub(crate) cartridge: persistence::CartridgeRuntimeState,
}

impl Core {
    pub fn new(cartridge_data: CartridgeData) -> Result<Core, Error> {
        Self::new_with_options(cartridge_data, CoreOptions::default())
    }

    pub fn new_with_options(
        cartridge_data: CartridgeData,
        options: CoreOptions,
    ) -> Result<Core, Error> {
        cartridge_data.validate()?;
        let mut cpu = Cpu::new();
        let cartridge = cartridge::try_from_with_options(cartridge_data, options)?;
        let apu = Apu::new(cpu.interrupt_mut());
        Ok(Self {
            cpu,
            ppu: Ppu::new(),
            apu,
            cartridge,
            options,
        })
    }

    pub fn reset(&mut self) {
        self.cpu.reset();
        self.ppu.reset();
        self.apu.reset(self.cpu.interrupt_mut());
    }

    pub fn peek_work_ram(&self, address: usize) -> Option<u8> {
        self.cpu.peek_work_ram(address)
    }

    pub fn peek_cartridge_ram(&self, address: usize) -> Option<OpenBusReadResult> {
        if (0x6000..=0x7FFF).contains(&address) {
            Some(self.cartridge.read(address))
        } else {
            None
        }
    }

    pub fn inspect_cartridge(
        cartridge_data: &CartridgeData,
        raw_file_len: usize,
    ) -> Result<RomInfo, Error> {
        let mapper = cartridge::try_from(cartridge_data.clone())?;
        let save_prg_ram_len = if cartridge_data.save_pram_length() > 0 {
            cartridge_data.save_pram_length()
        } else if cartridge_data.has_battery() {
            if cartridge_data.pram_length() > 0 {
                cartridge_data.pram_length()
            } else {
                mapper.save_len_default()
            }
        } else {
            0
        };
        Ok(RomInfo {
            format: cartridge_data.format(),
            mapper_type: cartridge_data.mapper_type(),
            sub_mapper_type: cartridge_data.sub_mapper_type(),
            mirror_mode: cartridge_data.mirror_mode(),
            has_battery: cartridge_data.has_battery(),
            trainer_len: cartridge_data.trainer().len(),
            prg_rom_len: cartridge_data.prog_rom_len(),
            chr_rom_len: cartridge_data.char_rom_len(),
            prg_ram_len: cartridge_data.pram_length(),
            save_prg_ram_len,
            chr_ram_len: cartridge_data.vram_length(),
            save_chr_ram_len: cartridge_data.save_vram_length(),
            raw_file_len,
            body_len: raw_file_len.saturating_sub(16),
        })
    }

    pub fn peek_ppu_vram(&self, address: usize) -> Option<u8> {
        self.ppu.peek_vram(address, self.cartridge.as_ref())
    }

    pub fn rom_identity(&self) -> RomIdentity {
        let data = self.cartridge.data_ref();
        RomIdentity {
            format: data.format(),
            mapper_type: data.mapper_type(),
            sub_mapper_type: data.sub_mapper_type(),
            mirror_mode: data.mirror_mode(),
            has_battery: data.has_battery(),
            trainer_len: data.trainer().len(),
            prg_rom_len: data.prog_rom_len(),
            chr_rom_len: data.char_rom_len(),
            prg_ram_len: data.pram_length(),
            save_prg_ram_len: data.save_pram_length(),
            chr_ram_len: data.vram_length(),
            save_chr_ram_len: data.save_vram_length(),
            prg_rom_crc64: crc64(data.prog_rom()),
            chr_rom_crc64: crc64(data.char_rom()),
            trainer_crc64: crc64(data.trainer()),
        }
    }

    pub fn options(&self) -> CoreOptions {
        self.options
    }

    pub fn has_persistent_mapper_save(&self) -> bool {
        self.cartridge.has_persistent_mapper_save()
    }

    pub fn export_mapper_save(&self) -> Result<Option<Vec<u8>>, Error> {
        if !self.has_persistent_mapper_save() {
            return Ok(None);
        }
        let (prg_ram, chr_ram) = self.cartridge.export_mapper_save_state()?;
        let payload = MapperSavePayload {
            schema_version: persistence::PERSISTENCE_SCHEMA_VERSION,
            rom_identity: self.rom_identity(),
            options: self.options,
            prg_ram,
            chr_ram,
        };
        Ok(Some(persistence::encode_payload(&payload)?))
    }

    pub fn import_mapper_save(&mut self, bytes: &[u8]) -> Result<(), Error> {
        let payload: MapperSavePayload = persistence::decode_payload(bytes)?;
        persistence::validate_schema_version(payload.schema_version)?;
        if payload.rom_identity != self.rom_identity() {
            return Err(
                persistence::PersistenceError::Validation("ROM identity mismatch".into()).into(),
            );
        }
        self.cartridge
            .import_mapper_save_state(&payload.prg_ram, &payload.chr_ram)?;
        Ok(())
    }

    pub fn export_machine_state(&self) -> Result<Vec<u8>, Error> {
        let payload = MachineStatePayload {
            schema_version: persistence::PERSISTENCE_SCHEMA_VERSION,
            rom_identity: self.rom_identity(),
            options: self.options,
            cpu: self.cpu.clone(),
            ppu: self.ppu.clone(),
            apu: self.apu.clone(),
            cartridge: self.cartridge.export_runtime_state()?,
        };
        Ok(persistence::encode_payload(&payload)?)
    }

    pub fn import_machine_state(&mut self, bytes: &[u8]) -> Result<(), Error> {
        let payload: MachineStatePayload = persistence::decode_payload(bytes)?;
        persistence::validate_schema_version(payload.schema_version)?;
        let rom_identity = payload.rom_identity;
        let options = payload.options;
        self.validate_persistence_target(rom_identity, options)?;
        let cpu = payload.cpu;
        cpu.validate_runtime_state()?;
        let ppu = payload.ppu;
        ppu.validate_runtime_state()?;
        let apu = payload.apu;
        apu.validate_runtime_state()?;
        self.cartridge.import_runtime_state(payload.cartridge)?;
        self.cpu = cpu;
        self.ppu = ppu;
        self.apu = apu;
        Ok(())
    }

    pub fn step<S: Screen, M: MixerInput>(
        &mut self,
        screen: &mut S,
        controller: &mut dyn Controller,
        mixer: &mut M,
    ) -> bool {
        self.step_cycle(screen, controller, mixer, mixer.sample_rate())
    }

    pub fn run_frame<S: Screen, M: MixerInput>(
        &mut self,
        screen: &mut S,
        controller: &mut dyn Controller,
        mixer: &mut M,
    ) -> u64 {
        let mut cycles = 0;
        let mixer_sample_rate = mixer.sample_rate();
        loop {
            cycles += 1;
            if self.step_cycle(screen, controller, mixer, mixer_sample_rate) {
                return cycles;
            }
        }
    }

    #[inline(always)]
    fn step_cycle<S: Screen, M: MixerInput>(
        &mut self,
        screen: &mut S,
        controller: &mut dyn Controller,
        mixer: &mut M,
        mixer_sample_rate: u32,
    ) -> bool {
        let mut result = false;
        self.cpu.step(
            &mut self.ppu,
            self.cartridge.as_mut(),
            controller,
            &mut self.apu,
        );
        for _ in 0..3 {
            let mut ppu_cartridge =
                crate::cartridge_bus::mapper_cartridge_bus(self.cartridge.as_mut());
            if self
                .ppu
                .step(screen, &mut ppu_cartridge, self.cpu.interrupt_mut())
            {
                result = true;
            }
        }
        self.cartridge.step(self.cpu.interrupt_mut());
        self.apu.step(
            &mut self.cpu,
            mixer,
            mixer_sample_rate,
            self.cartridge.expansion_audio_output(),
            self.cartridge.expansion_audio_inverted(),
        );

        result
    }

    fn validate_persistence_target(
        &self,
        identity: RomIdentity,
        options: CoreOptions,
    ) -> Result<(), persistence::PersistenceError> {
        if self.rom_identity() != identity {
            return Err(persistence::PersistenceError::Validation(
                "ROM identity mismatch".into(),
            ));
        }
        if self.options != options {
            return Err(persistence::PersistenceError::Validation(
                "runtime options mismatch".into(),
            ));
        }
        Ok(())
    }
}

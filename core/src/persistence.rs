// Copyright (c) 2024 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use crate::{CoreOptions, MirrorMode, Mmc3IrqVariant, RomFormat};
use prost::Message;
use thiserror::Error;

pub(crate) const PERSISTENCE_SCHEMA_VERSION: u32 = 1;

pub(crate) const MAPPER_KIND_NONE: &str = "";
pub(crate) const MAPPER_KIND_ACTION53: &str = "action53";
pub(crate) const MAPPER_KIND_FME7: &str = "fme7";
pub(crate) const MAPPER_KIND_MMC2: &str = "mmc2";
pub(crate) const MAPPER_KIND_MMC3: &str = "mmc3";
pub(crate) const MAPPER_KIND_MMC5: &str = "mmc5";
pub(crate) const MAPPER_KIND_SXROM: &str = "sxrom";

#[derive(Debug, Error)]
pub enum PersistenceError {
    #[error("protobuf decode failed: {0}")]
    Decode(#[from] prost::DecodeError),
    #[error("protobuf encode failed: {0}")]
    Encode(#[from] prost::EncodeError),
    #[error("invalid persistence payload: {0}")]
    Validation(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RomIdentity {
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
    pub prg_rom_crc64: u64,
    pub chr_rom_crc64: u64,
    pub trainer_crc64: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, prost::Enumeration)]
#[repr(i32)]
pub(crate) enum ProtoRomFormat {
    Ines = 0,
    Nes20 = 1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, prost::Enumeration)]
#[repr(i32)]
pub(crate) enum ProtoMmc3IrqVariant {
    Auto = 0,
    Sharp = 1,
    Nec = 2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, prost::Enumeration)]
#[repr(i32)]
pub(crate) enum ProtoMappingMode {
    Ram = 0,
    Rom = 1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, prost::Enumeration)]
#[repr(i32)]
pub(crate) enum ProtoMirrorModeKind {
    Horizontal = 0,
    Vertical = 1,
    Single0 = 2,
    Single1 = 3,
    Four = 4,
    Custom = 5,
}

#[derive(Clone, PartialEq, Message)]
pub(crate) struct RomIdentityMessage {
    #[prost(enumeration = "ProtoRomFormat", tag = "1")]
    pub format: i32,
    #[prost(uint32, tag = "2")]
    pub mapper_type: u32,
    #[prost(uint32, tag = "3")]
    pub sub_mapper_type: u32,
    #[prost(message, optional, tag = "4")]
    pub mirror_mode: Option<MirrorModeMessage>,
    #[prost(bool, tag = "5")]
    pub has_battery: bool,
    #[prost(uint64, tag = "6")]
    pub trainer_len: u64,
    #[prost(uint64, tag = "7")]
    pub prg_rom_len: u64,
    #[prost(uint64, tag = "8")]
    pub chr_rom_len: u64,
    #[prost(uint64, tag = "9")]
    pub prg_ram_len: u64,
    #[prost(uint64, tag = "10")]
    pub save_prg_ram_len: u64,
    #[prost(uint64, tag = "11")]
    pub chr_ram_len: u64,
    #[prost(uint64, tag = "12")]
    pub save_chr_ram_len: u64,
    #[prost(uint64, tag = "13")]
    pub prg_rom_crc64: u64,
    #[prost(uint64, tag = "14")]
    pub chr_rom_crc64: u64,
    #[prost(uint64, tag = "15")]
    pub trainer_crc64: u64,
}

#[derive(Clone, PartialEq, Message)]
pub(crate) struct CoreOptionsMessage {
    #[prost(enumeration = "ProtoMmc3IrqVariant", tag = "1")]
    pub mmc3_irq_variant: i32,
}

#[derive(Clone, PartialEq, Message)]
pub(crate) struct MirrorModeMessage {
    #[prost(enumeration = "ProtoMirrorModeKind", tag = "1")]
    pub kind: i32,
    #[prost(uint32, repeated, tag = "2")]
    pub custom_lut: Vec<u32>,
}

#[derive(Clone, PartialEq, Message)]
pub(crate) struct MapperPersistentMemoryMessage {
    #[prost(bytes = "vec", tag = "1")]
    pub prg_ram: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    pub chr_ram: Vec<u8>,
}

#[derive(Clone, PartialEq, Message)]
pub(crate) struct MapperStateMessage {
    #[prost(sint32, repeated, tag = "1")]
    pub program_page_table: Vec<i32>,
    #[prost(sint32, repeated, tag = "2")]
    pub character_page_table: Vec<i32>,
    #[prost(sint32, repeated, tag = "3")]
    pub sram_page_table: Vec<i32>,
    #[prost(bytes = "vec", tag = "4")]
    pub sram: Vec<u8>,
    #[prost(bytes = "vec", tag = "5")]
    pub vram: Vec<u8>,
    #[prost(message, optional, tag = "6")]
    pub mirror_mode: Option<MirrorModeMessage>,
    #[prost(bool, tag = "7")]
    pub has_battery: bool,
    #[prost(enumeration = "ProtoMappingMode", tag = "8")]
    pub character_mapping_mode: i32,
}

#[derive(Clone, PartialEq, Message)]
pub(crate) struct CartridgeRuntimeMessage {
    #[prost(message, optional, tag = "1")]
    pub mapper_state: Option<MapperStateMessage>,
    #[prost(string, tag = "2")]
    pub mapper_specific_kind: String,
    #[prost(bytes = "vec", tag = "3")]
    pub mapper_specific_body: Vec<u8>,
}

#[derive(Clone, PartialEq, Message)]
pub(crate) struct CpuMemoryMessage {
    #[prost(bytes = "vec", tag = "1")]
    pub wram: Vec<u8>,
    #[prost(uint32, tag = "2")]
    pub open_bus_data: u32,
}

#[derive(Clone, PartialEq, Message)]
pub(crate) struct RegisterMessage {
    #[prost(uint32, tag = "1")]
    pub pc: u32,
    #[prost(uint32, tag = "2")]
    pub sp: u32,
    #[prost(uint32, tag = "3")]
    pub a: u32,
    #[prost(uint32, tag = "4")]
    pub x: u32,
    #[prost(uint32, tag = "5")]
    pub y: u32,
    #[prost(uint32, tag = "6")]
    pub p: u32,
}

#[derive(Clone, PartialEq, Message)]
pub(crate) struct InternalStatMessage {
    #[prost(uint32, tag = "1")]
    pub opcode: u32,
    #[prost(uint32, tag = "2")]
    pub address: u32,
    #[prost(uint32, tag = "3")]
    pub step: u32,
    #[prost(uint32, tag = "4")]
    pub tempaddr: u32,
    #[prost(uint32, tag = "5")]
    pub data: u32,
    #[prost(bool, tag = "6")]
    pub crossed: bool,
    #[prost(bool, tag = "7")]
    pub interrupt: bool,
    #[prost(uint32, tag = "8")]
    pub state: u32,
}

#[derive(Clone, PartialEq, Message)]
pub(crate) struct InterruptMessage {
    #[prost(bool, tag = "1")]
    pub nmi: bool,
    #[prost(bool, tag = "2")]
    pub executing: bool,
    #[prost(bool, tag = "3")]
    pub detected: bool,
    #[prost(bool, tag = "4")]
    pub running_dma: bool,
    #[prost(uint32, tag = "5")]
    pub irq_mask: u32,
    #[prost(uint32, tag = "6")]
    pub irq_flag: u32,
    #[prost(uint32, optional, tag = "7")]
    pub oam_dma: Option<u32>,
    #[prost(uint32, optional, tag = "8")]
    pub dmc_dma_request: Option<u32>,
    #[prost(bool, tag = "9")]
    pub write: bool,
}

#[derive(Clone, PartialEq, Message)]
pub(crate) struct OamDmaStateValueMessage {
    #[prost(uint32, tag = "1")]
    pub offset: u32,
    #[prost(uint32, tag = "2")]
    pub count: u32,
    #[prost(uint32, tag = "3")]
    pub value: u32,
}

#[derive(Clone, PartialEq, Message)]
pub(crate) struct OamDmaStateMessage {
    #[prost(uint32, tag = "1")]
    pub state: u32,
    #[prost(message, optional, tag = "2")]
    pub value: Option<OamDmaStateValueMessage>,
}

#[derive(Clone, PartialEq, Message)]
pub(crate) struct DmcDmaStateMessage {
    #[prost(uint32, tag = "1")]
    pub delay: u32,
    #[prost(bool, tag = "2")]
    pub halt_on_get_cycle: bool,
    #[prost(bool, tag = "3")]
    pub halted_on_get_cycle: bool,
    #[prost(bool, tag = "4")]
    pub attempted_halt: bool,
    #[prost(uint32, tag = "5")]
    pub phase: u32,
}

#[derive(Clone, PartialEq, Message)]
pub(crate) struct CpuStateMessage {
    #[prost(message, optional, tag = "1")]
    pub memory: Option<CpuMemoryMessage>,
    #[prost(message, optional, tag = "2")]
    pub register: Option<RegisterMessage>,
    #[prost(message, optional, tag = "3")]
    pub internal_stat: Option<InternalStatMessage>,
    #[prost(message, optional, tag = "4")]
    pub interrupt: Option<InterruptMessage>,
    #[prost(uint64, tag = "5")]
    pub cycles: u64,
    #[prost(message, optional, tag = "6")]
    pub oam_dma: Option<OamDmaStateMessage>,
    #[prost(message, optional, tag = "7")]
    pub dmc_dma: Option<DmcDmaStateMessage>,
}

#[derive(Clone, PartialEq, Message)]
pub(crate) struct TileInfoMessage {
    #[prost(uint32, tag = "1")]
    pub low_byte: u32,
    #[prost(uint32, tag = "2")]
    pub high_byte: u32,
    #[prost(uint32, tag = "3")]
    pub palette_offset: u32,
    #[prost(uint32, tag = "4")]
    pub tile_addr: u32,
}

#[derive(Clone, PartialEq, Message)]
pub(crate) struct SpriteInfoMessage {
    #[prost(uint32, tag = "1")]
    pub low_byte: u32,
    #[prost(uint32, tag = "2")]
    pub high_byte: u32,
    #[prost(uint32, tag = "3")]
    pub palette_offset: u32,
    #[prost(uint32, tag = "4")]
    pub tile_addr: u32,
    #[prost(bool, tag = "5")]
    pub horizontal_mirror: bool,
    #[prost(bool, tag = "6")]
    pub priority: bool,
    #[prost(uint32, tag = "7")]
    pub position: u32,
}

#[derive(Clone, PartialEq, Message)]
pub(crate) struct DecayableOpenBusMessage {
    #[prost(uint32, tag = "1")]
    pub data: u32,
    #[prost(uint32, repeated, tag = "2")]
    pub decay: Vec<u32>,
}

#[derive(Clone, PartialEq, Message)]
pub(crate) struct PpuStateRegistersMessage {
    #[prost(uint32, tag = "1")]
    pub control: u32,
    #[prost(uint32, tag = "2")]
    pub mask: u32,
    #[prost(uint32, tag = "3")]
    pub oam_address: u32,
    #[prost(uint32, tag = "4")]
    pub vram_addr: u32,
    #[prost(uint32, tag = "5")]
    pub temp_vram_addr: u32,
    #[prost(uint32, tag = "6")]
    pub x_scroll: u32,
    #[prost(bool, tag = "7")]
    pub write_toggle: bool,
    #[prost(uint32, tag = "8")]
    pub high_bit_shift: u32,
    #[prost(uint32, tag = "9")]
    pub low_bit_shift: u32,
}

#[derive(Clone, PartialEq, Message)]
pub(crate) struct PpuControlMessage {
    #[prost(uint32, tag = "1")]
    pub name_table: u32,
    #[prost(bool, tag = "2")]
    pub increment: bool,
    #[prost(bool, tag = "3")]
    pub sprite_table: bool,
    #[prost(bool, tag = "4")]
    pub background_table: bool,
    #[prost(bool, tag = "5")]
    pub sprite_size: bool,
    #[prost(bool, tag = "6")]
    pub master_slave: bool,
    #[prost(bool, tag = "7")]
    pub nmi_output: bool,
}

#[derive(Clone, PartialEq, Message)]
pub(crate) struct PpuMaskMessage {
    #[prost(bool, tag = "1")]
    pub grayscale: bool,
    #[prost(bool, tag = "2")]
    pub show_left_background: bool,
    #[prost(bool, tag = "3")]
    pub show_left_sprites: bool,
    #[prost(bool, tag = "4")]
    pub show_background: bool,
    #[prost(bool, tag = "5")]
    pub show_sprites: bool,
    #[prost(bool, tag = "6")]
    pub red_tint: bool,
    #[prost(bool, tag = "7")]
    pub green_tint: bool,
    #[prost(bool, tag = "8")]
    pub blue_tint: bool,
}

#[derive(Clone, PartialEq, Message)]
pub(crate) struct PpuStatusMessage {
    #[prost(bool, tag = "1")]
    pub sprite_zero_hit: bool,
    #[prost(bool, tag = "2")]
    pub sprite_overflow: bool,
    #[prost(bool, tag = "3")]
    pub nmi_occurred: bool,
}

#[derive(Clone, PartialEq, Message)]
pub(crate) struct PpuStateMessage {
    #[prost(bytes = "vec", tag = "1")]
    pub vram: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    pub palette: Vec<u8>,
    #[prost(message, optional, tag = "3")]
    pub state: Option<PpuStateRegistersMessage>,
    #[prost(uint32, tag = "4")]
    pub cycle: u32,
    #[prost(uint32, tag = "5")]
    pub scan_line: u32,
    #[prost(uint64, tag = "6")]
    pub frames: u64,
    #[prost(uint64, tag = "7")]
    pub bus_tick: u64,
    #[prost(uint32, tag = "8")]
    pub buffered_data: u32,
    #[prost(bytes = "vec", tag = "9")]
    pub primary_oam: Vec<u8>,
    #[prost(bytes = "vec", tag = "10")]
    pub secondary_oam: Vec<u8>,
    #[prost(uint32, tag = "11")]
    pub secondary_oam_address: u32,
    #[prost(message, optional, tag = "12")]
    pub control: Option<PpuControlMessage>,
    #[prost(message, optional, tag = "13")]
    pub mask: Option<PpuMaskMessage>,
    #[prost(message, optional, tag = "14")]
    pub status: Option<PpuStatusMessage>,
    #[prost(message, optional, tag = "15")]
    pub current_tile: Option<TileInfoMessage>,
    #[prost(message, optional, tag = "16")]
    pub previous_tile: Option<TileInfoMessage>,
    #[prost(message, optional, tag = "17")]
    pub next_tile: Option<TileInfoMessage>,
    #[prost(message, repeated, tag = "18")]
    pub sprites: Vec<SpriteInfoMessage>,
    #[prost(uint32, tag = "19")]
    pub sprite_index: u32,
    #[prost(uint32, tag = "20")]
    pub sprite_count: u32,
    #[prost(bool, tag = "21")]
    pub render_executing: bool,
    #[prost(bool, tag = "22")]
    pub post_render_executing: bool,
    #[prost(uint32, tag = "23")]
    pub oam_read_buffer: u32,
    #[prost(uint32, tag = "24")]
    pub vram_read_delay: u32,
    #[prost(uint32, tag = "25")]
    pub vram_addr_update_delay: u32,
    #[prost(uint32, tag = "26")]
    pub new_vram_addr: u32,
    #[prost(bool, tag = "27")]
    pub has_first_sprite_next: bool,
    #[prost(bool, tag = "28")]
    pub has_first_sprite: bool,
    #[prost(bool, tag = "29")]
    pub has_sprite: bool,
    #[prost(uint32, tag = "30")]
    pub sprite_overflow_delay: u32,
    #[prost(bool, tag = "31")]
    pub sprite_reading: bool,
    #[prost(uint32, tag = "32")]
    pub oam_address_high: u32,
    #[prost(uint32, tag = "33")]
    pub oam_address_low: u32,
    #[prost(uint32, tag = "34")]
    pub openbus_vram_data: u32,
    #[prost(message, optional, tag = "35")]
    pub openbus_io: Option<DecayableOpenBusMessage>,
    #[prost(bool, tag = "36")]
    pub has_next_sprite: bool,
}

#[derive(Clone, PartialEq, Message)]
pub(crate) struct TimerDaoMessage {
    #[prost(uint32, tag = "1")]
    pub value: u32,
    #[prost(uint32, tag = "2")]
    pub period: u32,
}

#[derive(Clone, PartialEq, Message)]
pub(crate) struct LengthCounterDaoMessage {
    #[prost(bool, tag = "1")]
    pub next_halt: bool,
    #[prost(bool, tag = "2")]
    pub halt: bool,
    #[prost(bool, tag = "3")]
    pub enabled: bool,
    #[prost(uint32, tag = "4")]
    pub next_value: u32,
    #[prost(uint32, tag = "5")]
    pub value: u32,
    #[prost(uint32, tag = "6")]
    pub prev_value: u32,
}

#[derive(Clone, PartialEq, Message)]
pub(crate) struct EnvelopeDaoMessage {
    #[prost(bool, tag = "1")]
    pub enabled: bool,
    #[prost(uint32, tag = "2")]
    pub volume: u32,
    #[prost(bool, tag = "3")]
    pub start: bool,
    #[prost(uint32, tag = "4")]
    pub value: u32,
    #[prost(uint32, tag = "5")]
    pub period: u32,
}

#[derive(Clone, PartialEq, Message)]
pub(crate) struct PulseMessage {
    #[prost(bool, tag = "1")]
    pub is_first_channel: bool,
    #[prost(uint32, tag = "2")]
    pub duty_mode: u32,
    #[prost(uint32, tag = "3")]
    pub duty_value: u32,
    #[prost(bool, tag = "4")]
    pub sweep_reload: bool,
    #[prost(bool, tag = "5")]
    pub sweep_enabled: bool,
    #[prost(bool, tag = "6")]
    pub sweep_negate: bool,
    #[prost(uint32, tag = "7")]
    pub sweep_shift: u32,
    #[prost(uint32, tag = "8")]
    pub sweep_period: u32,
    #[prost(uint32, tag = "9")]
    pub sweep_value: u32,
    #[prost(uint32, tag = "10")]
    pub sweep_target_period: u32,
    #[prost(uint32, tag = "11")]
    pub period: u32,
    #[prost(message, optional, tag = "12")]
    pub envelope: Option<EnvelopeDaoMessage>,
    #[prost(message, optional, tag = "13")]
    pub length_counter: Option<LengthCounterDaoMessage>,
    #[prost(message, optional, tag = "14")]
    pub timer: Option<TimerDaoMessage>,
}

#[derive(Clone, PartialEq, Message)]
pub(crate) struct TriangleMessage {
    #[prost(uint32, tag = "1")]
    pub duty_value: u32,
    #[prost(uint32, tag = "2")]
    pub counter_period: u32,
    #[prost(uint32, tag = "3")]
    pub counter_value: u32,
    #[prost(bool, tag = "4")]
    pub counter_reload: bool,
    #[prost(bool, tag = "5")]
    pub counter_control: bool,
    #[prost(uint32, tag = "6")]
    pub output_value: u32,
    #[prost(message, optional, tag = "7")]
    pub length_counter: Option<LengthCounterDaoMessage>,
    #[prost(message, optional, tag = "8")]
    pub timer: Option<TimerDaoMessage>,
}

#[derive(Clone, PartialEq, Message)]
pub(crate) struct NoiseMessage {
    #[prost(bool, tag = "1")]
    pub mode: bool,
    #[prost(uint32, tag = "2")]
    pub shift_register: u32,
    #[prost(message, optional, tag = "3")]
    pub envelope: Option<EnvelopeDaoMessage>,
    #[prost(message, optional, tag = "4")]
    pub length_counter: Option<LengthCounterDaoMessage>,
    #[prost(message, optional, tag = "5")]
    pub timer: Option<TimerDaoMessage>,
}

#[derive(Clone, PartialEq, Message)]
pub(crate) struct DmcMessage {
    #[prost(uint32, tag = "1")]
    pub value: u32,
    #[prost(uint32, tag = "2")]
    pub sample_address: u32,
    #[prost(uint32, tag = "3")]
    pub sample_length: u32,
    #[prost(uint32, tag = "4")]
    pub length_value: u32,
    #[prost(uint32, tag = "5")]
    pub current_address: u32,
    #[prost(uint32, tag = "6")]
    pub shift_register: u32,
    #[prost(uint32, tag = "7")]
    pub bit_count: u32,
    #[prost(uint32, tag = "8")]
    pub read_buffer: u32,
    #[prost(bool, tag = "9")]
    pub enabled: bool,
    #[prost(bool, tag = "10")]
    pub need_buffer: bool,
    #[prost(bool, tag = "11")]
    pub is_loop: bool,
    #[prost(bool, tag = "12")]
    pub irq: bool,
    #[prost(message, optional, tag = "13")]
    pub timer: Option<TimerDaoMessage>,
}

#[derive(Clone, PartialEq, Message)]
pub(crate) struct FrameCounterMessage {
    #[prost(bool, tag = "1")]
    pub period: bool,
    #[prost(bool, tag = "2")]
    pub irq: bool,
    #[prost(uint32, tag = "3")]
    pub write_counter: u32,
    #[prost(uint32, tag = "4")]
    pub block: u32,
    #[prost(uint32, tag = "5")]
    pub new_value: u32,
    #[prost(uint64, tag = "6")]
    pub clock_cycle: u64,
    #[prost(uint32, tag = "7")]
    pub cycle: u32,
}

#[derive(Clone, PartialEq, Message)]
pub(crate) struct ApuStateMessage {
    #[prost(message, optional, tag = "1")]
    pub pulse1: Option<PulseMessage>,
    #[prost(message, optional, tag = "2")]
    pub pulse2: Option<PulseMessage>,
    #[prost(message, optional, tag = "3")]
    pub triangle: Option<TriangleMessage>,
    #[prost(message, optional, tag = "4")]
    pub noise: Option<NoiseMessage>,
    #[prost(message, optional, tag = "5")]
    pub dmc: Option<DmcMessage>,
    #[prost(uint64, tag = "6")]
    pub sample_accumulator: u64,
    #[prost(message, optional, tag = "7")]
    pub frame_counter: Option<FrameCounterMessage>,
}

#[derive(Clone, PartialEq, Message)]
pub(crate) struct MapperSavePayloadMessage {
    #[prost(uint32, tag = "1")]
    pub schema_version: u32,
    #[prost(message, optional, tag = "2")]
    pub rom_identity: Option<RomIdentityMessage>,
    #[prost(message, optional, tag = "3")]
    pub options: Option<CoreOptionsMessage>,
    #[prost(message, optional, tag = "4")]
    pub persistent_memory: Option<MapperPersistentMemoryMessage>,
}

#[derive(Clone, PartialEq, Message)]
pub(crate) struct MachineStatePayloadMessage {
    #[prost(uint32, tag = "1")]
    pub schema_version: u32,
    #[prost(message, optional, tag = "2")]
    pub rom_identity: Option<RomIdentityMessage>,
    #[prost(message, optional, tag = "3")]
    pub options: Option<CoreOptionsMessage>,
    #[prost(message, optional, tag = "4")]
    pub cpu: Option<CpuStateMessage>,
    #[prost(message, optional, tag = "5")]
    pub ppu: Option<PpuStateMessage>,
    #[prost(message, optional, tag = "6")]
    pub apu: Option<ApuStateMessage>,
    #[prost(message, optional, tag = "7")]
    pub cartridge: Option<CartridgeRuntimeMessage>,
}

pub(crate) fn encode_message<M: Message>(message: &M) -> Result<Vec<u8>, PersistenceError> {
    let mut bytes = Vec::new();
    message.encode(&mut bytes)?;
    Ok(bytes)
}

pub(crate) fn decode_message<M: Message + Default>(bytes: &[u8]) -> Result<M, PersistenceError> {
    Ok(M::decode(bytes)?)
}

pub(crate) fn validate_schema_version(version: u32) -> Result<(), PersistenceError> {
    if version == PERSISTENCE_SCHEMA_VERSION {
        Ok(())
    } else {
        Err(PersistenceError::Validation(format!(
            "unsupported persistence schema version: {version}"
        )))
    }
}

pub(crate) fn options_to_proto(options: CoreOptions) -> CoreOptionsMessage {
    CoreOptionsMessage {
        mmc3_irq_variant: match options.mmc3_irq_variant {
            Some(Mmc3IrqVariant::Sharp) => ProtoMmc3IrqVariant::Sharp,
            Some(Mmc3IrqVariant::Nec) => ProtoMmc3IrqVariant::Nec,
            None => ProtoMmc3IrqVariant::Auto,
        } as i32,
    }
}

pub(crate) fn options_from_proto(
    proto: &CoreOptionsMessage,
) -> Result<CoreOptions, PersistenceError> {
    Ok(CoreOptions {
        mmc3_irq_variant: match ProtoMmc3IrqVariant::try_from(proto.mmc3_irq_variant)
            .map_err(|_| PersistenceError::Validation("unknown MMC3 IRQ variant".into()))?
        {
            ProtoMmc3IrqVariant::Auto => None,
            ProtoMmc3IrqVariant::Sharp => Some(Mmc3IrqVariant::Sharp),
            ProtoMmc3IrqVariant::Nec => Some(Mmc3IrqVariant::Nec),
        },
    })
}

pub(crate) fn mirror_mode_to_proto(mode: MirrorMode) -> MirrorModeMessage {
    match mode {
        MirrorMode::Horizontal => MirrorModeMessage {
            kind: ProtoMirrorModeKind::Horizontal as i32,
            custom_lut: Vec::new(),
        },
        MirrorMode::Vertical => MirrorModeMessage {
            kind: ProtoMirrorModeKind::Vertical as i32,
            custom_lut: Vec::new(),
        },
        MirrorMode::Single0 => MirrorModeMessage {
            kind: ProtoMirrorModeKind::Single0 as i32,
            custom_lut: Vec::new(),
        },
        MirrorMode::Single1 => MirrorModeMessage {
            kind: ProtoMirrorModeKind::Single1 as i32,
            custom_lut: Vec::new(),
        },
        MirrorMode::Four => MirrorModeMessage {
            kind: ProtoMirrorModeKind::Four as i32,
            custom_lut: Vec::new(),
        },
        MirrorMode::Custom(lut) => MirrorModeMessage {
            kind: ProtoMirrorModeKind::Custom as i32,
            custom_lut: lut.into_iter().map(u32::from).collect(),
        },
    }
}

pub(crate) fn mirror_mode_from_proto(
    proto: &MirrorModeMessage,
) -> Result<MirrorMode, PersistenceError> {
    Ok(
        match ProtoMirrorModeKind::try_from(proto.kind)
            .map_err(|_| PersistenceError::Validation("unknown mirror mode".into()))?
        {
            ProtoMirrorModeKind::Horizontal => MirrorMode::Horizontal,
            ProtoMirrorModeKind::Vertical => MirrorMode::Vertical,
            ProtoMirrorModeKind::Single0 => MirrorMode::Single0,
            ProtoMirrorModeKind::Single1 => MirrorMode::Single1,
            ProtoMirrorModeKind::Four => MirrorMode::Four,
            ProtoMirrorModeKind::Custom => {
                if proto.custom_lut.len() != 4 {
                    return Err(PersistenceError::Validation(
                        "custom mirror mode LUT must contain 4 entries".into(),
                    ));
                }
                MirrorMode::Custom([
                    u8::try_from(proto.custom_lut[0]).map_err(|_| {
                        PersistenceError::Validation("custom mirror mode LUT overflow".into())
                    })?,
                    u8::try_from(proto.custom_lut[1]).map_err(|_| {
                        PersistenceError::Validation("custom mirror mode LUT overflow".into())
                    })?,
                    u8::try_from(proto.custom_lut[2]).map_err(|_| {
                        PersistenceError::Validation("custom mirror mode LUT overflow".into())
                    })?,
                    u8::try_from(proto.custom_lut[3]).map_err(|_| {
                        PersistenceError::Validation("custom mirror mode LUT overflow".into())
                    })?,
                ])
            }
        },
    )
}

pub(crate) fn rom_identity_to_proto(identity: RomIdentity) -> RomIdentityMessage {
    RomIdentityMessage {
        format: match identity.format {
            RomFormat::INes => ProtoRomFormat::Ines,
            RomFormat::Nes20 => ProtoRomFormat::Nes20,
        } as i32,
        mapper_type: u32::from(identity.mapper_type),
        sub_mapper_type: u32::from(identity.sub_mapper_type),
        mirror_mode: Some(mirror_mode_to_proto(identity.mirror_mode)),
        has_battery: identity.has_battery,
        trainer_len: identity.trainer_len as u64,
        prg_rom_len: identity.prg_rom_len as u64,
        chr_rom_len: identity.chr_rom_len as u64,
        prg_ram_len: identity.prg_ram_len as u64,
        save_prg_ram_len: identity.save_prg_ram_len as u64,
        chr_ram_len: identity.chr_ram_len as u64,
        save_chr_ram_len: identity.save_chr_ram_len as u64,
        prg_rom_crc64: identity.prg_rom_crc64,
        chr_rom_crc64: identity.chr_rom_crc64,
        trainer_crc64: identity.trainer_crc64,
    }
}

pub(crate) fn rom_identity_from_proto(
    proto: &RomIdentityMessage,
) -> Result<RomIdentity, PersistenceError> {
    Ok(RomIdentity {
        format: match ProtoRomFormat::try_from(proto.format)
            .map_err(|_| PersistenceError::Validation("unknown ROM format".into()))?
        {
            ProtoRomFormat::Ines => RomFormat::INes,
            ProtoRomFormat::Nes20 => RomFormat::Nes20,
        },
        mapper_type: u16::try_from(proto.mapper_type)
            .map_err(|_| PersistenceError::Validation("mapper type overflow".into()))?,
        sub_mapper_type: u8::try_from(proto.sub_mapper_type)
            .map_err(|_| PersistenceError::Validation("sub mapper type overflow".into()))?,
        mirror_mode: mirror_mode_from_proto(
            proto
                .mirror_mode
                .as_ref()
                .ok_or_else(|| PersistenceError::Validation("missing mirror mode".into()))?,
        )?,
        has_battery: proto.has_battery,
        trainer_len: usize::try_from(proto.trainer_len)
            .map_err(|_| PersistenceError::Validation("trainer length overflow".into()))?,
        prg_rom_len: usize::try_from(proto.prg_rom_len)
            .map_err(|_| PersistenceError::Validation("PRG ROM length overflow".into()))?,
        chr_rom_len: usize::try_from(proto.chr_rom_len)
            .map_err(|_| PersistenceError::Validation("CHR ROM length overflow".into()))?,
        prg_ram_len: usize::try_from(proto.prg_ram_len)
            .map_err(|_| PersistenceError::Validation("PRG RAM length overflow".into()))?,
        save_prg_ram_len: usize::try_from(proto.save_prg_ram_len)
            .map_err(|_| PersistenceError::Validation("save PRG RAM length overflow".into()))?,
        chr_ram_len: usize::try_from(proto.chr_ram_len)
            .map_err(|_| PersistenceError::Validation("CHR RAM length overflow".into()))?,
        save_chr_ram_len: usize::try_from(proto.save_chr_ram_len)
            .map_err(|_| PersistenceError::Validation("save CHR RAM length overflow".into()))?,
        prg_rom_crc64: proto.prg_rom_crc64,
        chr_rom_crc64: proto.chr_rom_crc64,
        trainer_crc64: proto.trainer_crc64,
    })
}

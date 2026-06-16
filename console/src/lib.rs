pub mod controller;
mod runner;
pub mod state;
pub mod video;

use self::runner::ConsoleRunner;
use self::runner::data::ConsoleData;
use self::runner::metrics::SharedConsoleMetrics;
use self::state::RuntimeStateExport;
use self::video::ConsoleVideo;
use crc::{CRC_64_XZ, Crc, Digest};
use nerust_cartridge_data::parse_cartridge_bytes;
use nerust_contract_core::audio::AudioBackend;
use nerust_contract_core::options::CoreOptions;
use nerust_contract_core::options::Mmc3IrqVariant;
use nerust_contract_core::persistence::CanonicalMediaIdentity;
use nerust_input_nes_runtime::ControllerState;
use nerust_nes_core::Core;
use nerust_nes_core::cartridge_rom::CartridgeData;
use nerust_screen_buffer::screen_buffer::ScreenBuffer;
use nerust_screen_filter::FilterType;
use nerust_screen_logical::LogicalSize;
use nerust_screen_physical::PhysicalSize;
use std::hash::Hasher;
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, RwLock};
use std::thread;
use std::thread::JoinHandle;
use thiserror::Error;

// The old crc crate exposed this reflected CRC-64/XZ variant as crc64::ECMA.
const CRC64_LEGACY_ECMA: Crc<u64> = Crc::<u64>::new(&CRC_64_XZ);

struct Crc64Hasher(Digest<'static, u64>);

impl Crc64Hasher {
    fn new() -> Self {
        Self(CRC64_LEGACY_ECMA.digest())
    }
}

impl Hasher for Crc64Hasher {
    fn write(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }

    fn finish(&self) -> u64 {
        self.0.clone().finalize()
    }
}

fn crc64(bytes: &[u8]) -> u64 {
    let mut hasher = Crc64Hasher::new();
    hasher.write(bytes);
    hasher.finish()
}

fn mmc3_irq_variant_label(value: Option<Mmc3IrqVariant>) -> &'static str {
    match value {
        Some(Mmc3IrqVariant::Sharp) => "sharp",
        Some(Mmc3IrqVariant::Nec) => "nec",
        None => "auto",
    }
}

fn print_rom_metadata(data: &[u8], cartridge_data: &CartridgeData, options: CoreOptions) {
    let body = data.get(16..).unwrap_or(&[]);
    let body_crc64 = crc64(body);

    match Core::inspect_cartridge(cartridge_data, data.len()) {
        Ok(info) => {
            println!(
                "ROM: body_crc64=0x{body_crc64:016X} format={} mapper={} submapper={} mirror={:?} battery={} trainer={} raw={} body={}",
                info.format.label(),
                info.mapper_type,
                info.sub_mapper_type,
                info.mirror_mode,
                info.has_battery,
                info.trainer_len,
                info.raw_file_len,
                info.body_len,
            );
            println!(
                "ROM memory: prg_rom={} chr_rom={} prg_ram={} save_prg_ram={} chr_ram={} save_chr_ram={} mmc3_irq_variant={}",
                info.prg_rom_len,
                info.chr_rom_len,
                info.prg_ram_len,
                info.save_prg_ram_len,
                info.chr_ram_len,
                info.save_chr_ram_len,
                mmc3_irq_variant_label(options.mmc3_irq_variant),
            );
        }
        Err(error) => {
            println!(
                "ROM: body_crc64=0x{body_crc64:016X} parse_error={error} mmc3_irq_variant={}",
                mmc3_irq_variant_label(options.mmc3_irq_variant),
            );
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ConsoleMetrics {
    pub frame_counter: u64,
    pub emulation_fps: f32,
    pub speed_multiplier: f32,
    pub loaded: bool,
    pub paused: bool,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ConsoleError {
    #[error("console worker thread is unavailable")]
    WorkerUnavailable,
    #[error("ROM parse failed: {0}")]
    RomParse(String),
    #[error("no ROM loaded")]
    NoRomLoaded,
    #[error("{0}")]
    Core(String),
}

enum ConsoleReply {
    Unit,
    MapperSave(Option<Vec<u8>>),
    CanonicalMediaIdentity(CanonicalMediaIdentity),
    StateExport(RuntimeStateExport),
}

type ConsoleRequestResult = Result<ConsoleReply, ConsoleError>;

#[derive(Debug)]
pub struct Console {
    stop_sender: Sender<()>,
    data_sender: Sender<ConsoleData>,
    thread: Option<JoinHandle<()>>,

    video: ConsoleVideo,
    metrics: SharedConsoleMetrics,
}

impl Console {
    pub fn new_gpu(
        speaker: Box<dyn AudioBackend>,
        filter_type: FilterType,
        source_logical_size: LogicalSize,
        controller: Box<dyn ControllerState>,
    ) -> Self {
        Self::new(
            speaker,
            ScreenBuffer::new_gpu(filter_type, source_logical_size),
            controller,
        )
    }

    pub fn new(
        speaker: Box<dyn AudioBackend>,
        screen_buffer: ScreenBuffer,
        controller: Box<dyn ControllerState>,
    ) -> Self {
        Self::spawn(speaker, screen_buffer, controller)
    }

    fn spawn(
        speaker: Box<dyn AudioBackend>,
        screen: ScreenBuffer,
        controller: Box<dyn ControllerState>,
    ) -> Self {
        let (data_sender, data_recv) = channel();
        let (stop_sender, stop_recv) = channel();
        let frame_len = screen.frame_len();
        let mut frame_buffer = vec![0; frame_len].into_boxed_slice();
        screen.write_frame_into(&mut frame_buffer);
        let frame_buffer = Arc::new(RwLock::new(frame_buffer));
        let metrics = SharedConsoleMetrics::new(ConsoleMetrics {
            paused: true,
            ..ConsoleMetrics::default()
        });

        let mut result = Self {
            data_sender,
            stop_sender,
            thread: None,
            video: ConsoleVideo::new(screen.video_presentation().clone(), frame_buffer.clone()),
            metrics: metrics.clone(),
        };

        result.thread = Some(thread::spawn(move || {
            let mut state = ConsoleRunner::new(
                data_recv,
                stop_recv,
                screen,
                frame_buffer,
                metrics,
                controller,
            );
            state.run(speaker);
        }));

        result
    }

    pub fn logical_size(&self) -> LogicalSize {
        self.video().render_profile().logical_size
    }

    pub fn source_logical_size(&self) -> LogicalSize {
        self.video().render_profile().source_logical_size
    }

    pub fn physical_size(&self) -> PhysicalSize {
        self.video().render_profile().physical_size
    }

    pub fn video(&self) -> &ConsoleVideo {
        &self.video
    }

    pub fn with_frame_buffer<T>(&self, f: impl FnOnce(&[u8]) -> T) -> T {
        self.video.with_frame_buffer(f)
    }

    pub fn metrics(&self) -> ConsoleMetrics {
        self.metrics.snapshot()
    }

    pub fn resume(&self) {
        if self.data_sender.send(ConsoleData::Resume).is_err() {
            log::warn!("Core resume send failed");
        }
    }

    pub fn pause(&self) {
        if self.data_sender.send(ConsoleData::Pause).is_err() {
            log::warn!("Core pause send failed");
        }
    }

    pub fn load(&self, data: Vec<u8>) -> Result<(), ConsoleError> {
        self.load_with_options(data, CoreOptions::default())
    }

    pub fn load_with_options(
        &self,
        data: Vec<u8>,
        options: CoreOptions,
    ) -> Result<(), ConsoleError> {
        match parse_cartridge_bytes(&data) {
            Ok(cartridge_data) => {
                print_rom_metadata(&data, &cartridge_data, options);
                self.send_request(|reply| ConsoleData::Load {
                    cartridge_data,
                    options,
                    reply,
                })?;
                Ok(())
            }
            Err(error) => {
                let body_crc64 = crc64(data.get(16..).unwrap_or(&[]));
                println!(
                    "ROM: body_crc64=0x{body_crc64:016X} parse_error={error} mmc3_irq_variant={}",
                    mmc3_irq_variant_label(options.mmc3_irq_variant),
                );
                Err(ConsoleError::RomParse(format!(
                    "body_crc64=0x{body_crc64:016X}: {error}"
                )))
            }
        }
    }

    pub fn unload(&self) -> Result<(), ConsoleError> {
        self.send_request(ConsoleData::Unload)?;
        Ok(())
    }

    pub fn reset(&self) -> Result<(), ConsoleError> {
        self.send_request(ConsoleData::Reset)?;
        Ok(())
    }

    pub fn export_mapper_save(&self) -> Result<Option<Vec<u8>>, ConsoleError> {
        match self.send_request(ConsoleData::ExportMapperSave)? {
            ConsoleReply::MapperSave(bytes) => Ok(bytes),
            _ => Err(ConsoleError::Core("unexpected mapper save reply".into())),
        }
    }

    pub fn import_mapper_save(&self, bytes: Vec<u8>) -> Result<(), ConsoleError> {
        self.send_request(|reply| ConsoleData::ImportMapperSave { bytes, reply })?;
        Ok(())
    }

    pub fn export_state(&self) -> Result<RuntimeStateExport, ConsoleError> {
        match self.send_request(ConsoleData::ExportState)? {
            ConsoleReply::StateExport(export) => Ok(export),
            _ => Err(ConsoleError::Core("unexpected state export reply".into())),
        }
    }

    pub fn canonical_media_identity(&self) -> Result<CanonicalMediaIdentity, ConsoleError> {
        match self.send_request(ConsoleData::CanonicalMediaIdentity)? {
            ConsoleReply::CanonicalMediaIdentity(identity) => Ok(identity),
            _ => Err(ConsoleError::Core(
                "unexpected canonical media identity reply".into(),
            )),
        }
    }

    pub fn import_state(&self, bytes: Vec<u8>) -> Result<(), ConsoleError> {
        self.send_request(|reply| ConsoleData::ImportState { bytes, reply })?;
        Ok(())
    }

    fn send_request(
        &self,
        build: impl FnOnce(Sender<ConsoleRequestResult>) -> ConsoleData,
    ) -> Result<ConsoleReply, ConsoleError> {
        let (reply_sender, reply_receiver) = channel();
        self.data_sender
            .send(build(reply_sender))
            .map_err(|_| ConsoleError::WorkerUnavailable)?;
        reply_receiver
            .recv()
            .map_err(|_| ConsoleError::WorkerUnavailable)?
    }
}

impl Drop for Console {
    fn drop(&mut self) {
        if self.stop_sender.send(()).is_err() {
            log::warn!("Core stop send failed");
        }
        let _ = self.thread.take().map(JoinHandle::join);
    }
}

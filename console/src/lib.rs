// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod video;

use crc::{CRC_64_XZ, Crc, Digest};
use nerust_cartridge_data::parse_cartridge_bytes;
use nerust_core::controller::standard_controller::{
    Buttons, StandardController, StandardControllerSnapshot,
};
use nerust_core::{CartridgeData, Core, CoreOptions, Mmc3IrqVariant, RomIdentity};
use nerust_screen_buffer::ScreenBuffer;
use nerust_screen_filter::FilterType;
use nerust_screen_traits::{LogicalSize, PhysicalSize};
use nerust_sound_traits::{MixerInput, Sound};
use nerust_timer::{TARGET_FPS, Timer};
use std::hash::{Hash, Hasher};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, RwLock};
use std::thread;
use std::thread::JoinHandle;
use thiserror::Error;
pub use video::ConsoleVideo;

// The old crc crate exposed this reflected CRC-64/XZ variant as crc64::ECMA.
const CRC64_LEGACY_ECMA: Crc<u64> = Crc::<u64>::new(&CRC_64_XZ);
const CONSOLE_STATE_SCHEMA_VERSION: u32 = 1;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewFrame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateExport {
    pub machine_state: Vec<u8>,
    pub preview: Option<PreviewFrame>,
    pub rom_identity: RomIdentity,
    pub options: CoreOptions,
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

#[derive(serde_derive::Serialize, serde_derive::Deserialize)]
struct ControllerStatePayload {
    pad1_bits: u32,
    pad2_bits: u32,
    microphone: bool,
    index1: u64,
    index2: u64,
    strobe: bool,
}

#[derive(serde_derive::Serialize, serde_derive::Deserialize)]
struct ConsoleStatePayload {
    schema_version: u32,
    #[serde(with = "serde_bytes")]
    core_state: Vec<u8>,
    frame_counter: u64,
    paused: bool,
    controller: ControllerStatePayload,
    rom_identity: RomIdentity,
    options: CoreOptions,
    #[serde(with = "serde_bytes")]
    source_frame: Vec<u8>,
}

enum ConsoleReply {
    Unit,
    MapperSave(Option<Vec<u8>>),
    PersistenceTarget(RomIdentity, CoreOptions),
    StateExport(StateExport),
}

type ConsoleRequestResult = Result<ConsoleReply, ConsoleError>;

#[derive(Debug)]
pub struct Console {
    stop_sender: Sender<()>,
    data_sender: Sender<ConsoleData>,
    thread: Option<JoinHandle<()>>,

    video: ConsoleVideo,
    metrics: Arc<RwLock<ConsoleMetrics>>,
}

impl Console {
    pub fn new_gpu<S: 'static + Sound + MixerInput + Send>(
        speaker: S,
        filter_type: FilterType,
        source_logical_size: LogicalSize,
    ) -> Self {
        Self::new(
            speaker,
            ScreenBuffer::new_gpu(filter_type, source_logical_size),
        )
    }

    pub fn new<S: 'static + Sound + MixerInput + Send>(
        speaker: S,
        screen_buffer: ScreenBuffer,
    ) -> Self {
        Self::spawn(speaker, screen_buffer)
    }

    fn spawn<S: 'static + Sound + MixerInput + Send>(speaker: S, screen: ScreenBuffer) -> Self {
        let (data_sender, data_recv) = channel();
        let (stop_sender, stop_recv) = channel();
        let mut frame_buffer = vec![0; screen.frame_len()].into_boxed_slice();
        screen.copy_frame_buffer(frame_buffer.as_mut());
        let frame_buffer = Arc::new(RwLock::new(frame_buffer));
        let metrics = Arc::new(RwLock::new(ConsoleMetrics {
            paused: true,
            ..ConsoleMetrics::default()
        }));

        let mut result = Self {
            data_sender,
            stop_sender,
            thread: None,
            video: ConsoleVideo::new(screen.video_presentation().clone(), frame_buffer.clone()),
            metrics: metrics.clone(),
        };

        result.thread = Some(thread::spawn(move || {
            let mut state = ConsoleRunner::new(data_recv, stop_recv, screen, frame_buffer, metrics);
            state.run(speaker);
        }));

        result
    }

    pub fn logical_size(&self) -> LogicalSize {
        self.video().presentation().logical_size()
    }

    pub fn source_logical_size(&self) -> LogicalSize {
        self.video().presentation().source_logical_size()
    }

    pub fn physical_size(&self) -> PhysicalSize {
        self.video().presentation().physical_size()
    }

    pub fn video(&self) -> &ConsoleVideo {
        &self.video
    }

    pub fn with_frame_buffer<T>(&self, f: impl FnOnce(&[u8]) -> T) -> T {
        self.video.frame_buffer().with_bytes(f)
    }

    pub fn metrics(&self) -> ConsoleMetrics {
        *self.metrics.read().unwrap_or_else(|err| err.into_inner())
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

    pub fn set_pad1(&self, data: Buttons) {
        if self.data_sender.send(ConsoleData::Pad1Data(data)).is_err() {
            log::warn!("Core pad1 data send failed");
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

    pub fn export_state(&self) -> Result<StateExport, ConsoleError> {
        match self.send_request(ConsoleData::ExportState)? {
            ConsoleReply::StateExport(export) => Ok(export),
            _ => Err(ConsoleError::Core("unexpected state export reply".into())),
        }
    }

    pub fn persistence_target(&self) -> Result<(RomIdentity, CoreOptions), ConsoleError> {
        match self.send_request(ConsoleData::PersistenceTarget)? {
            ConsoleReply::PersistenceTarget(rom_identity, options) => Ok((rom_identity, options)),
            _ => Err(ConsoleError::Core(
                "unexpected persistence target reply".into(),
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

fn controller_snapshot_to_payload(snapshot: StandardControllerSnapshot) -> ControllerStatePayload {
    ControllerStatePayload {
        pad1_bits: u32::from(snapshot.buttons[0].bits()),
        pad2_bits: u32::from(snapshot.buttons[1].bits()),
        microphone: snapshot.microphone,
        index1: snapshot.index1 as u64,
        index2: snapshot.index2 as u64,
        strobe: snapshot.strobe,
    }
}

fn controller_snapshot_from_payload(
    payload: &ControllerStatePayload,
) -> Result<StandardControllerSnapshot, ConsoleError> {
    Ok(StandardControllerSnapshot {
        buttons: [
            Buttons::from_bits_retain(
                u8::try_from(payload.pad1_bits)
                    .map_err(|_| ConsoleError::Core("controller pad1 overflow".into()))?,
            ),
            Buttons::from_bits_retain(
                u8::try_from(payload.pad2_bits)
                    .map_err(|_| ConsoleError::Core("controller pad2 overflow".into()))?,
            ),
        ],
        microphone: payload.microphone,
        index1: usize::try_from(payload.index1)
            .map_err(|_| ConsoleError::Core("controller index1 overflow".into()))?,
        index2: usize::try_from(payload.index2)
            .map_err(|_| ConsoleError::Core("controller index2 overflow".into()))?,
        strobe: payload.strobe,
    })
}

enum ConsoleData {
    Load {
        cartridge_data: CartridgeData,
        options: CoreOptions,
        reply: Sender<ConsoleRequestResult>,
    },
    Resume,
    Pause,
    Reset(Sender<ConsoleRequestResult>),
    Pad1Data(Buttons),
    Unload(Sender<ConsoleRequestResult>),
    ExportMapperSave(Sender<ConsoleRequestResult>),
    ImportMapperSave {
        bytes: Vec<u8>,
        reply: Sender<ConsoleRequestResult>,
    },
    PersistenceTarget(Sender<ConsoleRequestResult>),
    ExportState(Sender<ConsoleRequestResult>),
    ImportState {
        bytes: Vec<u8>,
        reply: Sender<ConsoleRequestResult>,
    },
}

struct ConsoleRunner {
    timer: Timer,
    controller: StandardController,
    paused: bool,
    frame_counter: u64,

    stop_receiver: Receiver<()>,
    data_receiver: Receiver<ConsoleData>,
    screen: ScreenBuffer,
    frame_buffer: Arc<RwLock<Box<[u8]>>>,
    metrics: Arc<RwLock<ConsoleMetrics>>,
}

impl ConsoleRunner {
    pub(crate) fn new(
        data_receiver: Receiver<ConsoleData>,
        stop_receiver: Receiver<()>,
        screen: ScreenBuffer,
        frame_buffer: Arc<RwLock<Box<[u8]>>>,
        metrics: Arc<RwLock<ConsoleMetrics>>,
    ) -> Self {
        Self {
            data_receiver,
            stop_receiver,

            timer: Timer::new(),
            controller: StandardController::new(),
            paused: true,
            frame_counter: 0,
            screen,
            frame_buffer,
            metrics,
        }
    }

    fn publish_frame(&self) {
        let mut frame_buffer = self
            .frame_buffer
            .write()
            .unwrap_or_else(|err| err.into_inner());
        self.screen.copy_frame_buffer(frame_buffer.as_mut());
    }

    fn publish_metrics(&self, loaded: bool) {
        let emulation_fps = if loaded && !self.paused {
            self.timer.as_fps()
        } else {
            0.0
        };
        let speed_multiplier = if emulation_fps > 0.0 {
            emulation_fps / TARGET_FPS
        } else {
            0.0
        };
        let mut metrics = self.metrics.write().unwrap_or_else(|err| err.into_inner());
        *metrics = ConsoleMetrics {
            frame_counter: self.frame_counter,
            emulation_fps,
            speed_multiplier,
            loaded,
            paused: self.paused,
        };
    }

    fn export_preview_frame(&self) -> Option<PreviewFrame> {
        let palette = self.screen.video_presentation().palette_rgba8()?;
        let source_size = self.screen.source_logical_size();
        let mut indices = vec![0; self.screen.source_frame_len()];
        self.screen.copy_source_buffer(&mut indices);
        let mut rgba = Vec::with_capacity(indices.len() * 4);
        for index in indices {
            let palette_index = usize::from(index) * 4;
            let pixel = palette.get(palette_index..palette_index + 4)?;
            rgba.extend_from_slice(pixel);
        }
        Some(PreviewFrame {
            width: source_size.width as u32,
            height: source_size.height as u32,
            rgba,
        })
    }

    fn reply(reply: Sender<ConsoleRequestResult>, result: Result<ConsoleReply, ConsoleError>) {
        if reply.send(result).is_err() {
            log::warn!("console reply send failed");
        }
    }

    fn core_not_loaded() -> ConsoleError {
        ConsoleError::NoRomLoaded
    }

    fn run<S: Sound + MixerInput>(&mut self, mut speaker: S) {
        let mut core: Option<Core> = None;
        while self.stop_receiver.try_recv().is_err() {
            if let Some(core) = core.as_mut()
                && !self.paused
            {
                core.run_frame(&mut self.screen, &mut self.controller, &mut speaker);
                self.frame_counter += 1;
                self.publish_frame();
            }
            self.timer.wait();
            self.publish_metrics(core.is_some());
            if let Ok(event) = self.data_receiver.try_recv() {
                match event {
                    ConsoleData::Load {
                        cartridge_data,
                        options,
                        reply,
                    } => {
                        let result = Core::new_with_options(cartridge_data, options)
                            .map_err(|error| ConsoleError::Core(error.to_string()));
                        match result {
                            Ok(new_core) => {
                                self.screen.clear();
                                self.publish_frame();
                                self.frame_counter = 0;
                                core = Some(new_core);
                                Self::reply(reply, Ok(ConsoleReply::Unit));
                            }
                            Err(error) => Self::reply(reply, Err(error)),
                        }
                    }
                    ConsoleData::Resume => {
                        self.paused = false;
                        speaker.start();
                    }
                    ConsoleData::Pause => {
                        self.paused = true;
                        speaker.pause();
                        let mut hasher = Crc64Hasher::new();
                        self.screen.hash(&mut hasher);
                        log::info!(
                            "Paused -- FrameCounter : {}, ScreenHash : 0x{:016X}",
                            self.frame_counter,
                            hasher.finish()
                        );
                    }
                    ConsoleData::Reset(reply) => {
                        let result = if let Some(core) = core.as_mut() {
                            core.reset();
                            self.frame_counter = 0;
                            Ok(ConsoleReply::Unit)
                        } else {
                            Err(Self::core_not_loaded())
                        };
                        Self::reply(reply, result);
                    }
                    ConsoleData::Pad1Data(data) => {
                        self.controller.set_pad1(data);
                    }
                    ConsoleData::Unload(reply) => {
                        let result = if core.is_some() {
                            self.paused = false;
                            self.frame_counter = 0;
                            core = None;
                            self.screen.clear();
                            self.publish_frame();
                            Ok(ConsoleReply::Unit)
                        } else {
                            Err(Self::core_not_loaded())
                        };
                        Self::reply(reply, result);
                    }
                    ConsoleData::ExportMapperSave(reply) => {
                        let result =
                            core.as_ref()
                                .ok_or_else(Self::core_not_loaded)
                                .and_then(|core| {
                                    core.export_mapper_save()
                                        .map(ConsoleReply::MapperSave)
                                        .map_err(|error| ConsoleError::Core(error.to_string()))
                                });
                        Self::reply(reply, result);
                    }
                    ConsoleData::ImportMapperSave { bytes, reply } => {
                        let result =
                            core.as_mut()
                                .ok_or_else(Self::core_not_loaded)
                                .and_then(|core| {
                                    core.import_mapper_save(&bytes)
                                        .map(|_| ConsoleReply::Unit)
                                        .map_err(|error| ConsoleError::Core(error.to_string()))
                                });
                        Self::reply(reply, result);
                    }
                    ConsoleData::PersistenceTarget(reply) => {
                        let result = core.as_ref().ok_or_else(Self::core_not_loaded).map(|core| {
                            ConsoleReply::PersistenceTarget(core.rom_identity(), core.options())
                        });
                        Self::reply(reply, result);
                    }
                    ConsoleData::ExportState(reply) => {
                        let preview = self.export_preview_frame();
                        let result =
                            core.as_ref()
                                .ok_or_else(Self::core_not_loaded)
                                .and_then(|core| {
                                    let machine_state = core
                                        .export_machine_state()
                                        .map_err(|error| ConsoleError::Core(error.to_string()))?;
                                    let source_frame = if self.screen.publishes_palette_frame() {
                                        let mut source_frame =
                                            vec![0; self.screen.source_frame_len()];
                                        self.screen.copy_source_buffer(&mut source_frame);
                                        source_frame
                                    } else {
                                        Vec::new()
                                    };
                                    let state = ConsoleStatePayload {
                                        schema_version: CONSOLE_STATE_SCHEMA_VERSION,
                                        core_state: machine_state,
                                        frame_counter: self.frame_counter,
                                        paused: self.paused,
                                        controller: controller_snapshot_to_payload(
                                            self.controller.export_snapshot(),
                                        ),
                                        rom_identity: core.rom_identity(),
                                        options: core.options(),
                                        source_frame,
                                    };
                                    Ok(ConsoleReply::StateExport(StateExport {
                                        machine_state: rmp_serde::to_vec_named(&state).map_err(
                                            |error| ConsoleError::Core(error.to_string()),
                                        )?,
                                        preview,
                                        rom_identity: core.rom_identity(),
                                        options: core.options(),
                                    }))
                                });
                        Self::reply(reply, result);
                    }
                    ConsoleData::ImportState { bytes, reply } => {
                        let result =
                            core.as_mut()
                                .ok_or_else(Self::core_not_loaded)
                                .and_then(|core| {
                                    let payload = rmp_serde::from_slice::<ConsoleStatePayload>(
                                        bytes.as_slice(),
                                    )
                                    .map_err(|error| {
                                        ConsoleError::Core(format!(
                                            "console state decode failed: {error}"
                                        ))
                                    })?;
                                    if payload.schema_version != CONSOLE_STATE_SCHEMA_VERSION {
                                        return Err(ConsoleError::Core(format!(
                                            "unsupported console state schema version: {}",
                                            payload.schema_version
                                        )));
                                    }
                                    let controller =
                                        controller_snapshot_from_payload(&payload.controller)?;
                                    if self.screen.publishes_palette_frame()
                                        && !payload.source_frame.is_empty()
                                        && payload.source_frame.len()
                                            != self.screen.source_frame_len()
                                    {
                                        return Err(ConsoleError::Core(
                                            "console source frame length mismatch".into(),
                                        ));
                                    }
                                    core.import_machine_state(&payload.core_state)
                                        .map_err(|error| ConsoleError::Core(error.to_string()))?;
                                    if self.screen.publishes_palette_frame()
                                        && !payload.source_frame.is_empty()
                                    {
                                        self.screen.restore_source_buffer(&payload.source_frame);
                                    }
                                    self.controller.import_snapshot(controller);
                                    self.frame_counter = payload.frame_counter;
                                    self.paused = payload.paused;
                                    if self.paused {
                                        speaker.pause();
                                    } else {
                                        speaker.start();
                                    }
                                    self.publish_frame();
                                    Ok(ConsoleReply::Unit)
                                });
                        self.publish_metrics(core.is_some());
                        Self::reply(reply, result);
                    }
                }
                self.publish_metrics(core.is_some());
            }
        }
    }
}

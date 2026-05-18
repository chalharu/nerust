// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use crc::{CRC_64_XZ, Crc, Digest};
use nerust_core::controller::standard_controller::{Buttons, StandardController};
use nerust_core::{Core, CoreOptions, Mmc3IrqVariant};
use nerust_screen_buffer::ScreenBuffer;
use nerust_screen_traits::LogicalSize;
use nerust_sound_traits::{MixerInput, Sound};
use nerust_timer::{TARGET_FPS, Timer};
use std::hash::{Hash, Hasher};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, RwLock};
use std::thread;
use std::thread::JoinHandle;

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

fn print_rom_metadata(data: &[u8], options: CoreOptions) {
    let body = data.get(16..).unwrap_or(&[]);
    let body_crc64 = crc64(body);

    match Core::inspect_rom(data) {
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

#[derive(Debug)]
pub struct Console {
    stop_sender: Sender<()>,
    data_sender: Sender<ConsoleData>,
    thread: Option<JoinHandle<()>>,

    logical_size: LogicalSize,
    frame_buffer: Arc<RwLock<Box<[u8]>>>,
    metrics: Arc<RwLock<ConsoleMetrics>>,
}

impl Console {
    pub fn new<S: 'static + Sound + MixerInput + Send>(
        speaker: S,
        screen_buffer: ScreenBuffer,
    ) -> Self {
        let (data_sender, data_recv) = channel();
        let (stop_sender, stop_recv) = channel();
        let logical_size = screen_buffer.logical_size();
        let mut frame_buffer = vec![0; screen_buffer.frame_len()].into_boxed_slice();
        screen_buffer.copy_display_buffer(frame_buffer.as_mut());
        let frame_buffer = Arc::new(RwLock::new(frame_buffer));
        let metrics = Arc::new(RwLock::new(ConsoleMetrics {
            paused: true,
            ..ConsoleMetrics::default()
        }));

        let mut result = Self {
            data_sender,
            stop_sender,
            thread: None,
            logical_size,
            frame_buffer: frame_buffer.clone(),
            metrics: metrics.clone(),
        };

        result.thread = Some(thread::spawn(move || {
            let mut state =
                ConsoleRunner::new(data_recv, stop_recv, screen_buffer, frame_buffer, metrics);

            state.run(speaker);
        }));

        result
    }

    pub fn logical_size(&self) -> LogicalSize {
        self.logical_size
    }

    pub fn with_frame_buffer<T>(&self, f: impl FnOnce(&[u8]) -> T) -> T {
        let frame_buffer = self
            .frame_buffer
            .read()
            .unwrap_or_else(|err| err.into_inner());
        f(&frame_buffer)
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

    pub fn load(&self, data: Vec<u8>) {
        self.load_with_options(data, CoreOptions::default());
    }

    pub fn load_with_options(&self, data: Vec<u8>, options: CoreOptions) {
        print_rom_metadata(&data, options);
        if self
            .data_sender
            .send(ConsoleData::Load { data, options })
            .is_err()
        {
            log::warn!("Core load send failed");
        }
    }

    pub fn unload(&self) {
        if self.data_sender.send(ConsoleData::Unload).is_err() {
            log::warn!("Core unload send failed");
        }
    }

    pub fn reset(&self) {
        if self.data_sender.send(ConsoleData::Reset).is_err() {
            log::warn!("Core reset send failed");
        }
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

enum ConsoleData {
    Load { data: Vec<u8>, options: CoreOptions },
    Resume,
    Pause,
    Reset,
    Pad1Data(Buttons),
    Unload,
}

struct ConsoleRunner {
    timer: Timer,
    controller: StandardController,
    paused: bool,
    frame_counter: u64,

    stop_receiver: Receiver<()>,
    data_receiver: Receiver<ConsoleData>,
    screen_buffer: ScreenBuffer,
    frame_buffer: Arc<RwLock<Box<[u8]>>>,
    metrics: Arc<RwLock<ConsoleMetrics>>,
}

impl ConsoleRunner {
    pub(crate) fn new(
        data_receiver: Receiver<ConsoleData>,
        stop_receiver: Receiver<()>,
        screen_buffer: ScreenBuffer,
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
            screen_buffer,
            frame_buffer,
            metrics,
        }
    }

    fn publish_frame(&self) {
        let mut frame_buffer = self
            .frame_buffer
            .write()
            .unwrap_or_else(|err| err.into_inner());
        self.screen_buffer
            .copy_display_buffer(frame_buffer.as_mut());
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

    fn run<S: Sound + MixerInput>(&mut self, mut speaker: S) {
        let mut core: Option<Core> = None;
        while self.stop_receiver.try_recv().is_err() {
            if let Some(core) = core.as_mut()
                && !self.paused
            {
                core.run_frame(&mut self.screen_buffer, &mut self.controller, &mut speaker);
                self.frame_counter += 1;
                self.publish_frame();
            }
            self.timer.wait();
            self.publish_metrics(core.is_some());
            if let Ok(event) = self.data_receiver.try_recv() {
                match event {
                    ConsoleData::Load { data, options } => {
                        self.screen_buffer.clear();
                        self.publish_frame();
                        self.frame_counter = 0;
                        core = Core::new_with_options(&mut data.into_iter(), options).ok();
                    }
                    ConsoleData::Resume => {
                        self.paused = false;
                        speaker.start();
                    }
                    ConsoleData::Pause => {
                        self.paused = true;
                        speaker.pause();
                        let mut hasher = Crc64Hasher::new();
                        self.screen_buffer.hash(&mut hasher);
                        log::info!(
                            "Paused -- FrameCounter : {}, ScreenHash : 0x{:016X}",
                            self.frame_counter,
                            hasher.finish()
                        );
                    }
                    ConsoleData::Reset => {
                        core.as_mut().map(Core::reset).unwrap();
                        self.frame_counter = 0;
                    }
                    ConsoleData::Pad1Data(data) => {
                        self.controller.set_pad1(data);
                    }
                    ConsoleData::Unload => {
                        self.paused = false;
                        self.frame_counter = 0;
                        core = None;
                        self.screen_buffer.clear();
                        self.publish_frame();
                    } // _ => (),
                }
                self.publish_metrics(core.is_some());
            }
        }
    }
}

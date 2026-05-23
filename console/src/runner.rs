use super::{
    AuxiliaryInput, ConsoleError, ConsoleMetrics, ConsoleReply, ConsoleRequestResult,
    ControllerInputs, ControllerPort, Crc64Hasher,
};
use crate::{CoreOptions, PersistenceTarget, state};
use nerust_core::controller::standard_controller::StandardController;
use nerust_core::{CartridgeData, Core};
use nerust_screen_buffer::ScreenBuffer;
use nerust_sound_traits::{MixerInput, Sound};
use nerust_timer::{TARGET_FPS, Timer};
use std::hash::{Hash, Hasher};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, RwLock};

pub(super) enum ConsoleData {
    Load {
        cartridge_data: CartridgeData,
        options: CoreOptions,
        reply: Sender<ConsoleRequestResult>,
    },
    Resume,
    Pause,
    Reset(Sender<ConsoleRequestResult>),
    PortInputs {
        port: ControllerPort,
        inputs: ControllerInputs,
    },
    AuxiliaryInput {
        input: AuxiliaryInput,
        active: bool,
    },
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

pub(super) struct ConsoleRunner {
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
    pub(super) fn new(
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

    fn reply(reply: Sender<ConsoleRequestResult>, result: Result<ConsoleReply, ConsoleError>) {
        if reply.send(result).is_err() {
            log::warn!("console reply send failed");
        }
    }

    fn core_not_loaded() -> ConsoleError {
        ConsoleError::NoRomLoaded
    }

    pub(super) fn run<S: Sound + MixerInput>(&mut self, mut speaker: S) {
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
                    ConsoleData::PortInputs { port, inputs } => match port {
                        ControllerPort::One => {
                            self.controller
                                .set_pad1(state::buttons_from_controller_inputs(inputs));
                        }
                        ControllerPort::Two => {
                            self.controller
                                .set_pad2(state::buttons_from_controller_inputs(inputs));
                        }
                    },
                    ConsoleData::AuxiliaryInput { input, active } => match input {
                        AuxiliaryInput::Microphone => {
                            self.controller.set_microphone(active);
                        }
                    },
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
                            ConsoleReply::PersistenceTarget(PersistenceTarget {
                                rom_identity: core.rom_identity(),
                                options: core.options(),
                            })
                        });
                        Self::reply(reply, result);
                    }
                    ConsoleData::ExportState(reply) => {
                        let result =
                            core.as_ref()
                                .ok_or_else(Self::core_not_loaded)
                                .and_then(|core| {
                                    state::build_state_export(
                                        core,
                                        &self.screen,
                                        &self.controller,
                                        self.frame_counter,
                                        self.paused,
                                    )
                                    .map(ConsoleReply::StateExport)
                                });
                        Self::reply(reply, result);
                    }
                    ConsoleData::ImportState { bytes, reply } => {
                        let result =
                            core.as_mut()
                                .ok_or_else(Self::core_not_loaded)
                                .and_then(|core| {
                                    state::restore_imported_state(
                                        core,
                                        &mut self.screen,
                                        &mut self.controller,
                                        &mut self.frame_counter,
                                        &mut self.paused,
                                        &bytes,
                                    )?;
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

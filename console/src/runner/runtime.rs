use super::{ConsoleData, ConsoleRunner};
use crate::{ConsoleError, ConsoleReply, ConsoleRequestResult, Crc64Hasher};
use nerust_core::Core;
use nerust_sound_traits::{MixerInput, Sound};
use std::hash::{Hash, Hasher};
use std::sync::mpsc::Sender;

impl ConsoleRunner {
    fn apply_input_state(&mut self, bytes: &[u8]) -> Result<(), ConsoleError> {
        self.controller
            .apply_input_state(bytes)
            .map_err(ConsoleError::Core)
    }

    fn apply_controller_state(&mut self, bytes: &[u8]) -> Result<(), ConsoleError> {
        self.controller
            .apply_controller_state(bytes)
            .map_err(ConsoleError::Core)
    }

    fn current_controller_state(&self) -> Result<Vec<u8>, ConsoleError> {
        self.controller
            .current_controller_state()
            .map_err(ConsoleError::Core)
    }

    fn current_input_state(&self) -> Result<Vec<u8>, ConsoleError> {
        self.controller
            .current_input_state()
            .map_err(ConsoleError::Core)
    }

    fn reply(reply: Sender<ConsoleRequestResult>, result: Result<ConsoleReply, ConsoleError>) {
        if reply.send(result).is_err() {
            log::warn!("console reply send failed");
        }
    }

    pub(crate) fn core_not_loaded() -> ConsoleError {
        ConsoleError::NoRomLoaded
    }

    pub(crate) fn run<S: Sound + MixerInput>(&mut self, mut speaker: S) {
        let mut core: Option<Core> = None;
        while self.stop_receiver.try_recv().is_err() {
            if let Some(core) = core.as_mut()
                && !self.paused
            {
                core.run_frame(&mut self.screen, self.controller.as_mut(), &mut speaker);
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
                                self.controller.reset_runtime();
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
                    ConsoleData::ApplyInputState { bytes } => {
                        if let Err(error) = self.apply_input_state(&bytes) {
                            log::error!("input state apply failed: {error}");
                        }
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
                    ConsoleData::ApplyControllerState { bytes, reply } => {
                        let result = self
                            .apply_controller_state(&bytes)
                            .map(|_| ConsoleReply::Unit);
                        Self::reply(reply, result);
                    }
                    ConsoleData::Unload(reply) => {
                        let result = if core.is_some() {
                            self.paused = false;
                            self.frame_counter = 0;
                            self.controller.reset_runtime();
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
                        let result = self.export_mapper_save_reply(core.as_ref());
                        Self::reply(reply, result);
                    }
                    ConsoleData::ImportMapperSave { bytes, reply } => {
                        let result = self.import_mapper_save_reply(core.as_mut(), &bytes);
                        Self::reply(reply, result);
                    }
                    ConsoleData::CanonicalMediaIdentity(reply) => {
                        let result = self.canonical_media_identity_reply(core.as_ref());
                        Self::reply(reply, result);
                    }
                    ConsoleData::ExportState(reply) => {
                        let result = self.export_state_reply(core.as_ref());
                        Self::reply(reply, result);
                    }
                    ConsoleData::CurrentControllerState(reply) => {
                        Self::reply(
                            reply,
                            self.current_controller_state()
                                .map(ConsoleReply::ControllerState),
                        );
                    }
                    ConsoleData::CurrentInputState(reply) => {
                        Self::reply(
                            reply,
                            self.current_input_state().map(ConsoleReply::InputState),
                        );
                    }
                    ConsoleData::ImportState { bytes, reply } => {
                        let result = self.import_state_reply(core.as_mut(), &bytes);
                        if result.is_ok() {
                            if self.paused {
                                speaker.pause();
                            } else {
                                speaker.start();
                            }
                            self.publish_frame();
                        }
                        Self::reply(reply, result);
                    }
                }
                self.publish_metrics(core.is_some());
            }
        }
    }
}

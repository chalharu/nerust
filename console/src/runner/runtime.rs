use super::{ConsoleData, ConsoleRunner};
use crate::{ConsoleError, ConsoleReply, ConsoleRequestResult, Crc64Hasher, NesInputFrame, state};
use nerust_core::Core;
use nerust_sound_traits::{MixerInput, Sound};
use std::hash::{Hash, Hasher};
use std::sync::mpsc::Sender;

impl ConsoleRunner {
    fn apply_nes_input_frame(&mut self, frame: NesInputFrame) {
        self.controller
            .set_pad(state::buttons_from_nes_input_frame(frame));
        self.controller.set_microphone(frame.microphone);
    }

    fn current_nes_input_frame(&self) -> NesInputFrame {
        state::nes_input_frame_from_snapshot(self.controller.export_snapshot())
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
                    ConsoleData::NesInputFrame { frame } => self.apply_nes_input_frame(frame),
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
                        let result = self.export_mapper_save_reply(core.as_ref());
                        Self::reply(reply, result);
                    }
                    ConsoleData::ImportMapperSave { bytes, reply } => {
                        let result = self.import_mapper_save_reply(core.as_mut(), &bytes);
                        Self::reply(reply, result);
                    }
                    ConsoleData::PersistenceTarget(reply) => {
                        let result = self.persistence_target_reply(core.as_ref());
                        Self::reply(reply, result);
                    }
                    ConsoleData::ExportState(reply) => {
                        let result = self.export_state_reply(core.as_ref());
                        Self::reply(reply, result);
                    }
                    ConsoleData::CurrentNesInputFrame(reply) => {
                        Self::reply(
                            reply,
                            Ok(ConsoleReply::NesInputFrame(self.current_nes_input_frame())),
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

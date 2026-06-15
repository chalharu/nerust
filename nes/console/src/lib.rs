use nerust_cartridge_data::parse_cartridge_bytes;
use nerust_nes_core::Core;
use nerust_nes_core::OpenBusReadResult;
use nerust_nes_core::controller::Controller;

use nerust_contract_core::device::Device;
use nerust_contract_core::options::CoreOptions;
use nerust_contract_core::{
    ConsoleCore, CoreCapabilities, CoreConfig, CoreError, GpuCommand, GpuCommandList,
    VideoSignalKind,
};
use nerust_screen_buffer::screen_buffer::ScreenBuffer;
use nerust_screen_video::PixelFormat;
use nerust_sound_traits::MixerInput;

pub struct NesConsoleCore {
    core: Option<Core>,
    screen: ScreenBuffer,
    ctrl: PadController,
    mixer: Box<dyn MixerInput + Send>,
}

// Core is not Send, but NesConsoleCore only runs inside EmuThread
unsafe impl Send for NesConsoleCore {}

impl NesConsoleCore {
    pub fn new(screen: ScreenBuffer, mixer: Box<dyn MixerInput + Send>) -> Self {
        Self {
            core: None,
            screen,
            ctrl: PadController::new(),
            mixer,
        }
    }
}

impl ConsoleCore for NesConsoleCore {
    fn capabilities(&self) -> CoreCapabilities {
        let fmt = PixelFormat::Rgba;
        CoreCapabilities {
            output_formats: vec![fmt],
            video_signal: VideoSignalKind::Ntsc,
        }
    }

    fn apply_input_state(&mut self, bytes: &[u8]) {
        if let Ok(frame) = nerust_input_nes::codec::decode_input_state(bytes) {
            self.ctrl
                .set_buttons(frame.player_one.bits(), frame.player_two.bits());
        }
    }

    fn render_frame(&mut self, frame_slot: &mut [u8]) -> Result<GpuCommandList, CoreError> {
        let core = self.core.as_mut().ok_or(CoreError::NoRomLoaded)?;
        core.run_frame(&mut self.screen, &mut self.ctrl, &mut *self.mixer);
        // screen.render() is called by PPU inside run_frame

        self.screen.write_frame_into(frame_slot);

        Ok(GpuCommandList {
            commands: vec![GpuCommand::Blit { slot: 0 }],
        })
    }

    fn attach_device(&mut self, _port: usize, _device: Box<dyn Device>) {}
    fn detach_device(&mut self, _port: usize) {}

    fn load(&mut self, rom: &[u8], _config: &CoreConfig) -> Result<(), CoreError> {
        let cartridge_data =
            parse_cartridge_bytes(rom).map_err(|e| CoreError::RomParse(e.to_string()))?;
        self.screen.clear();
        self.core = Some(
            Core::new_with_options(cartridge_data, CoreOptions::default())
                .map_err(|e| CoreError::Core(e.to_string()))?,
        );
        Ok(())
    }

    fn unload(&mut self) {
        self.core = None;
    }

    fn reset(&mut self) {
        if let Some(core) = self.core.as_mut() {
            core.reset();
        }
    }

    fn paused(&self) -> bool {
        false
    }
    fn set_paused(&mut self, _paused: bool) {}

    fn save_state(&self) -> Result<Vec<u8>, CoreError> {
        self.core
            .as_ref()
            .ok_or(CoreError::NoRomLoaded)?
            .export_machine_state()
            .map_err(|e| CoreError::Core(e.to_string()))
    }

    fn load_state(&mut self, data: &[u8]) -> Result<(), CoreError> {
        self.core
            .as_mut()
            .ok_or(CoreError::NoRomLoaded)?
            .import_machine_state(data)
            .map_err(|e| CoreError::Core(e.to_string()))
    }
}

// --- PadController: NES controller shift register ---

struct PadController {
    buttons1: u8,
    buttons2: u8,
    index1: u8,
    index2: u8,
    strobe: bool,
}

impl PadController {
    fn new() -> Self {
        Self {
            buttons1: 0,
            buttons2: 0,
            index1: 0,
            index2: 0,
            strobe: false,
        }
    }

    fn set_buttons(&mut self, p1: u8, p2: u8) {
        self.buttons1 = p1;
        self.buttons2 = p2;
    }
}

impl Controller for PadController {
    fn read(&mut self, address: usize) -> OpenBusReadResult {
        match address {
            0 => {
                let bit = if self.index1 < 8 {
                    (self.buttons1 >> self.index1) & 1
                } else {
                    1
                };
                if !self.strobe {
                    self.index1 += 1;
                }
                OpenBusReadResult::new(bit, 7)
            }
            _ => {
                let bit = if self.index2 < 8 {
                    (self.buttons2 >> self.index2) & 1
                } else {
                    1
                };
                if !self.strobe {
                    self.index2 += 1;
                }
                OpenBusReadResult::new(bit, 0x1F)
            }
        }
    }

    fn write(&mut self, value: u8) {
        self.strobe = value & 1 == 1;
        if self.strobe {
            self.index1 = 0;
            self.index2 = 0;
        }
    }
}



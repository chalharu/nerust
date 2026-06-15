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
const NES_PALETTE: [[u8; 4]; 64] = [
    [102, 102, 102, 0xFF],
    [0, 42, 136, 0xFF],
    [20, 18, 167, 0xFF],
    [59, 0, 164, 0xFF],
    [92, 0, 126, 0xFF],
    [110, 0, 64, 0xFF],
    [108, 6, 0, 0xFF],
    [86, 29, 0, 0xFF],
    [51, 53, 0, 0xFF],
    [11, 72, 0, 0xFF],
    [0, 82, 0, 0xFF],
    [0, 79, 8, 0xFF],
    [0, 64, 77, 0xFF],
    [0, 0, 0, 0xFF],
    [0, 0, 0, 0xFF],
    [0, 0, 0, 0xFF],
    [173, 173, 173, 0xFF],
    [21, 95, 217, 0xFF],
    [66, 64, 255, 0xFF],
    [117, 39, 254, 0xFF],
    [160, 26, 204, 0xFF],
    [183, 30, 123, 0xFF],
    [181, 49, 32, 0xFF],
    [153, 78, 0, 0xFF],
    [107, 109, 0, 0xFF],
    [56, 135, 0, 0xFF],
    [12, 147, 0, 0xFF],
    [0, 143, 50, 0xFF],
    [0, 124, 141, 0xFF],
    [0, 0, 0, 0xFF],
    [0, 0, 0, 0xFF],
    [0, 0, 0, 0xFF],
    [255, 254, 255, 0xFF],
    [100, 176, 255, 0xFF],
    [146, 144, 255, 0xFF],
    [198, 118, 255, 0xFF],
    [243, 106, 255, 0xFF],
    [254, 110, 204, 0xFF],
    [254, 129, 112, 0xFF],
    [234, 158, 34, 0xFF],
    [188, 190, 0, 0xFF],
    [136, 216, 0, 0xFF],
    [92, 228, 48, 0xFF],
    [69, 224, 130, 0xFF],
    [72, 205, 222, 0xFF],
    [79, 79, 79, 0xFF],
    [0, 0, 0, 0xFF],
    [0, 0, 0, 0xFF],
    [255, 254, 255, 0xFF],
    [192, 223, 255, 0xFF],
    [211, 210, 255, 0xFF],
    [232, 200, 255, 0xFF],
    [251, 194, 255, 0xFF],
    [254, 196, 234, 0xFF],
    [254, 204, 197, 0xFF],
    [247, 216, 165, 0xFF],
    [228, 229, 148, 0xFF],
    [207, 239, 150, 0xFF],
    [189, 244, 171, 0xFF],
    [179, 243, 204, 0xFF],
    [181, 235, 242, 0xFF],
    [184, 184, 184, 0xFF],
    [0, 0, 0, 0xFF],
    [0, 0, 0, 0xFF],
];

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

        let src = self.screen.front_frame();
        let src_data = src.as_ref();
        let pixel_count = src.width() * src.height();
        for (i, &index) in src_data.iter().enumerate().take(pixel_count) {
            let rgba = &NES_PALETTE[index as usize % 64];
            let pos = i * 4;
            if pos + 4 > frame_slot.len() {
                break;
            }
            frame_slot[pos..pos + 4].copy_from_slice(rgba);
        }

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



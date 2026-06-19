use nerust_contract_core::audio::AudioBackend;
use nerust_contract_core::device::Device;
use nerust_contract_core::{
    ConsoleCore, CoreCapabilities, CoreConfig, CoreError, FrameBuffer, GpuCommand, GpuCommandList,
    PixelFormat, VideoSignalKind,
};

use crate::cartridge_rom::CartridgeData;
use crate::{Controller, Core};

/// Core は内部的に `Box<dyn Cartridge>` を持つが、全ての具象 mapper は Send。
struct SendCore(Option<Core>);

unsafe impl Send for SendCore {}

fn cartridge_error_to_core(e: crate::cartridge_error::CartridgeError) -> CoreError {
    CoreError::Core(e.to_string())
}

pub struct NesConsoleCore {
    core: SendCore,
    controller: Box<dyn Controller + Send>,
    audio: Box<dyn AudioBackend>,
    paused: bool,
}

impl NesConsoleCore {
    pub fn new(
        cartridge_data: CartridgeData,
        controller: Box<dyn Controller + Send>,
        audio: Box<dyn AudioBackend>,
    ) -> Result<Self, CoreError> {
        let core = Core::new(cartridge_data).map_err(|e| CoreError::Core(e.to_string()))?;
        Ok(Self {
            core: SendCore(Some(core)),
            controller,
            audio,
            paused: false,
        })
    }
}

impl ConsoleCore for NesConsoleCore {
    fn capabilities(&self) -> CoreCapabilities {
        CoreCapabilities {
            output_formats: vec![PixelFormat::PaletteIndex {
                palette: Box::new([0u32; 256]),
            }],
            video_signal: VideoSignalKind::Ntsc,
        }
    }

    fn render_frame(&mut self, frame_slot: &mut FrameBuffer) -> Result<GpuCommandList, CoreError> {
        let core = self.core.0.as_mut().ok_or(CoreError::NoRomLoaded)?;

        core.run_frame(frame_slot, self.controller.as_mut(), self.audio.as_mut());

        Ok(GpuCommandList {
            commands: vec![GpuCommand::PaletteDecode { slot: 0 }],
        })
    }

    fn attach_device(&mut self, port: usize, _device: Box<dyn Device>) {
        log::warn!("NesConsoleCore::attach_device: port {port} not implemented yet");
    }

    fn detach_device(&mut self, port: usize) {
        log::warn!("NesConsoleCore::detach_device: port {port} not implemented yet");
    }

    fn load(&mut self, rom: &[u8], _config: &CoreConfig) -> Result<(), CoreError> {
        use crate::cartridge_rom::parse_rom;
        let cartridge_data = parse_rom(rom).map_err(cartridge_error_to_core)?;
        let core = Core::new(cartridge_data).map_err(|e| CoreError::Core(e.to_string()))?;
        self.core = SendCore(Some(core));
        self.paused = false;
        Ok(())
    }

    fn unload(&mut self) {
        self.core = SendCore(None);
        self.paused = false;
    }

    fn reset(&mut self) {
        if let Some(core) = self.core.0.as_mut() {
            core.reset();
        }
    }

    fn paused(&self) -> bool {
        self.paused
    }

    fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    fn save_state(&self) -> Result<Vec<u8>, CoreError> {
        let core = self.core.0.as_ref().ok_or(CoreError::NoRomLoaded)?;
        core.export_machine_state()
            .map_err(|e| CoreError::Core(e.to_string()))
    }

    fn load_state(&mut self, data: &[u8]) -> Result<(), CoreError> {
        let core = self.core.0.as_mut().ok_or(CoreError::NoRomLoaded)?;
        core.import_machine_state(data)
            .map_err(|e| CoreError::Core(e.to_string()))
    }
}

use nerust_contract_core::audio::AudioBackend;
use nerust_contract_core::device::{Device, PortIo};
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
    devices: Vec<Option<Box<dyn Device>>>,
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
            devices: (0..2).map(|_| None).collect(),
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

        for device in self.devices.iter_mut().flatten() {
            let kind = device.kind();
            device.cycle(&mut PortIo {
                device: kind,
                input: Vec::new(),
                output: Vec::new(),
            });
        }

        // controller はコンストラクタで注入されたものを使用する。
        // attach_device で追加された Device は Phase 7 まで run_frame の
        // &mut dyn Controller に変換できない（trait upcasting 制約）。
        core.run_frame(frame_slot, self.controller.as_mut(), self.audio.as_mut());

        Ok(GpuCommandList {
            commands: vec![GpuCommand::PaletteDecode { slot: 0 }],
        })
    }

    /// デバイスを指定 port に取り付ける。
    ///
    /// 注: Phase 7 までは Device → Controller bridging が未実装のため、
    /// 取り付けられた Device は cycle() の呼び出し対象にはなるが、
    /// run_frame に渡す controller はコンストラクタのものが使われ続ける。
    /// Pad1/Pad2 はコンストラクタ経由で接続される。
    fn attach_device(&mut self, port: usize, device: Box<dyn Device>) {
        if port >= self.devices.len() {
            self.devices.resize_with(port + 1, || None);
        }
        self.devices[port] = Some(device);
    }

    fn detach_device(&mut self, port: usize) {
        if port < self.devices.len() {
            self.devices[port] = None;
        }
    }

    fn load(&mut self, rom: &[u8], _config: &CoreConfig) -> Result<(), CoreError> {
        let cartridge_data = crate::rom_parse::parse_rom(rom).map_err(cartridge_error_to_core)?;
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

    fn set_volume(&mut self, volume: f32) {
        self.audio.set_volume(volume);
    }

    fn mapper_save(&self) -> Result<Option<Vec<u8>>, CoreError> {
        let core = self.core.0.as_ref().ok_or(CoreError::NoRomLoaded)?;
        core.export_mapper_save()
            .map_err(|e| CoreError::Core(e.to_string()))
    }

    fn import_mapper_save(&mut self, data: &[u8]) -> Result<(), CoreError> {
        let core = self.core.0.as_mut().ok_or(CoreError::NoRomLoaded)?;
        core.import_mapper_save(data)
            .map_err(|e| CoreError::Core(e.to_string()))
    }

    fn identity(
        &self,
    ) -> Result<nerust_contract_core::persistence::CanonicalMediaIdentity, CoreError> {
        let core = self.core.0.as_ref().ok_or(CoreError::NoRomLoaded)?;
        Ok(nerust_contract_core::persistence::CanonicalMediaIdentity::Rom(core.rom_identity()))
    }
}

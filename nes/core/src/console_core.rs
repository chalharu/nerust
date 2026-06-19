use std::array;

use nerust_contract_core::audio::AudioBackend;
use nerust_contract_core::device::{Device, DeviceKind, PortIo};
use nerust_contract_core::persistence::CanonicalMediaIdentity;
use nerust_contract_core::{
    ConsoleCore, CoreCapabilities, CoreConfig, CoreError, FrameBuffer, GpuCommand, GpuCommandList,
    PixelFormat, VideoSignalKind,
};

use crate::cartridge_rom::CartridgeData;
use crate::{Controller, Core};

const NUM_PORTS: usize = 2;

/// `Core` は `pub(crate)` な `Cartridge` trait (`Box<dyn Cartridge>`) を含む。
/// 全ての具象 mapper は同一 crate 内 (`nes/core/src/cartridge/mapper/`) にあり、
/// かつ全て Send であることが確認されているため、`unsafe impl Send` は安全。
struct SendCore(Option<Core>);

// Safety: 全ての Cartridge 実装は同一 crate 内にあり、全て Send。
// `pub(crate)` なので外部からの非 Send 実装追加は不可能。
unsafe impl Send for SendCore {}

fn cartridge_error_to_core(e: crate::cartridge_error::CartridgeError) -> CoreError {
    CoreError::Core(e.to_string())
}

pub struct NesConsoleCore {
    core: SendCore,
    controller: Box<dyn Controller + Send>,
    audio: Box<dyn AudioBackend>,
    devices: [Option<Box<dyn Device>>; NUM_PORTS],
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
            devices: array::from_fn(|_| None),
            paused: false,
        })
    }

    /// Creates a NesConsoleCore with no ROM loaded.
    /// Call `load()` before `render_frame()`.
    pub fn new_empty(controller: Box<dyn Controller + Send>, audio: Box<dyn AudioBackend>) -> Self {
        Self {
            core: SendCore(None),
            controller,
            audio,
            devices: array::from_fn(|_| None),
            paused: false,
        }
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

        let mut port_io = PortIo {
            device: DeviceKind::None,
            input: Vec::new(),
            output: Vec::new(),
        };
        for device in self.devices.iter_mut().flatten() {
            port_io.device = device.kind();
            device.cycle(&mut port_io);
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
        if let Some(slot) = self.devices.get_mut(port) {
            *slot = Some(device);
        } else {
            log::warn!("NesConsoleCore: port {port} out of range (max {NUM_PORTS})");
        }
    }

    fn detach_device(&mut self, port: usize) {
        if let Some(slot) = self.devices.get_mut(port) {
            *slot = None;
        }
    }

    // TODO(Phase 7): `CoreConfig` から `CoreOptions` を抽出し、
    // `Core::new_with_options(cartridge_data, options)` を使う。
    // `region` フィールドは NES PAL 対応時に使用する。
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

    fn identity(&self) -> Result<CanonicalMediaIdentity, CoreError> {
        let core = self.core.0.as_ref().ok_or(CoreError::NoRomLoaded)?;
        Ok(CanonicalMediaIdentity::Rom(core.rom_identity()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OpenBusReadResult;
    use crate::controller::Controller;
    use nerust_contract_core::CoreConfig;
    use nerust_screen_video::PixelFormat;
    use std::collections::HashMap;

    struct MockController;
    impl Controller for MockController {
        fn read(&mut self, _address: usize) -> OpenBusReadResult {
            OpenBusReadResult::new(0, 0)
        }
        fn write(&mut self, _value: u8) {}
    }

    fn test_rom() -> Vec<u8> {
        let mut rom = vec![
            0x4E, 0x45, 0x53, 0x1A, 0x02, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ];
        rom.resize(16 + 0x8000 + 0x2000, 0);
        rom
    }

    #[test]
    fn load_and_render_frame() {
        let rom = test_rom();

        // parse_rom should succeed
        let cartridge = crate::rom_parse::parse_rom(&rom).expect("parse_rom should succeed");
        assert_eq!(cartridge.mapper_type(), 0);

        // NesConsoleCore::new should succeed
        let mut core = NesConsoleCore::new(
            cartridge,
            Box::new(MockController),
            Box::new(nerust_contract_core::audio::NullAudio),
        )
        .expect("NesConsoleCore::new should succeed");

        // render_frame should succeed
        let mut fb = FrameBuffer::with_capacity(
            256,
            240,
            PixelFormat::PaletteIndex {
                palette: Box::new([0u32; 256]),
            },
        );
        let result = core.render_frame(&mut fb);
        assert!(result.is_ok(), "render_frame should succeed: {:?}", result);
    }

    #[test]
    fn load_via_trait_method() {
        let rom = test_rom();
        let mut core = NesConsoleCore::new_empty(
            Box::new(MockController),
            Box::new(nerust_contract_core::audio::NullAudio),
        );
        let config = CoreConfig {
            region: None,
            bios_paths: HashMap::new(),
            controllers: HashMap::new(),
        };

        // load should succeed via trait method
        let result = ConsoleCore::load(&mut core, &rom, &config);
        assert!(result.is_ok(), "load should succeed: {:?}", result);

        // render_frame should succeed after load
        let mut fb = FrameBuffer::with_capacity(
            256,
            240,
            PixelFormat::PaletteIndex {
                palette: Box::new([0u32; 256]),
            },
        );
        let result = core.render_frame(&mut fb);
        assert!(
            result.is_ok(),
            "render_frame after load should succeed: {:?}",
            result
        );
    }
}

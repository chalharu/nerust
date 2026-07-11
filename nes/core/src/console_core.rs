use std::any::Any;

use crate::input_types::NesInputBuffer;
use nerust_core_traits::{
    ConsoleCore, CoreCapabilities, CoreConfig, CoreError, FrameBuffer, PixelFormat,
    VideoSignalKind, audio::AudioBackend, identity::SystemIdentity,
};
use nerust_input_traits::EmuInput;

use crate::{
    Core,
    cartridge_rom::CartridgeData,
    controller::{ControllerCollection, ControllerHub},
    core_options::CoreOptions,
};

/// `Core` は `pub(crate)` な `Cartridge` trait (`Box<dyn Cartridge>`) を含む。
/// 全ての具象 mapper は同一 crate 内 (`nes/core/src/cartridge/mapper/`) にあり、
/// かつ全て Send であることが確認されているため、`unsafe impl Send` は安全。
struct SendCore(Option<Core>);

// Safety: 全ての Cartridge 実装は同一 crate 内にあり、全て Send。
// `pub(crate)` なので外部からの非 Send 実装追加は不可能。
unsafe impl Send for SendCore {}

pub struct NesConsoleCore {
    core: SendCore,
    controller: ControllerCollection,
    audio: Box<dyn AudioBackend>,
    emu_input: EmuInput,
    paused: bool,
}

impl NesConsoleCore {
    pub fn new(
        cartridge_data: CartridgeData,
        controller: ControllerCollection,
        audio: Box<dyn AudioBackend>,
        emu_input: EmuInput,
    ) -> Result<Self, CoreError> {
        let core = Core::new(cartridge_data).map_err(CoreError::Core)?;
        Ok(Self {
            core: SendCore(Some(core)),
            controller,
            audio,
            emu_input,
            paused: false,
        })
    }

    /// Creates a NesConsoleCore with no ROM loaded.
    /// Call `load()` before `render_frame()`.
    pub fn new_empty(
        controller: ControllerCollection,
        audio: Box<dyn AudioBackend>,
        emu_input: EmuInput,
    ) -> Self {
        Self {
            core: SendCore(None),
            controller,
            audio,
            emu_input,
            paused: false,
        }
    }
}

impl NesConsoleCore {
    fn core_ref(&self) -> Result<&Core, CoreError> {
        self.core.0.as_ref().ok_or(CoreError::NoRomLoaded)
    }

    fn core_mut(&mut self) -> Result<&mut Core, CoreError> {
        self.core.0.as_mut().ok_or(CoreError::NoRomLoaded)
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

    fn render_frame(&mut self, frame_slot: &mut FrameBuffer) -> Result<(), CoreError> {
        let core = self.core.0.as_mut().ok_or(CoreError::NoRomLoaded)?;

        // Take latest input and sync to controller
        self.emu_input.take();
        let any: &dyn Any = &*self.emu_input.read_buf;
        if let Some(state) = any.downcast_ref::<NesInputBuffer>() {
            self.controller.sync_input(&state.0);
        }

        core.run_frame(frame_slot, &mut self.controller, self.audio.as_mut());

        Ok(())
    }

    // `region` フィールドは NES PAL 対応時に使用する。
    fn load(&mut self, rom: &[u8], config: &CoreConfig) -> Result<(), CoreError> {
        let cartridge_data =
            crate::rom_parse::parse_rom(rom).map_err(|e| CoreError::RomParse(Box::new(e)))?;
        let options = if config.core_options.is_empty() {
            CoreOptions::default()
        } else {
            CoreOptions::from_bytes(&config.core_options).unwrap_or_default()
        };
        let core = Core::new_with_options(cartridge_data, options).map_err(CoreError::Core)?;
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
        let core = self.core_ref()?;
        core.export_machine_state().map_err(CoreError::Core)
    }

    fn load_state(&mut self, data: &[u8]) -> Result<(), CoreError> {
        let core = self.core_mut()?;
        core.import_machine_state(data).map_err(CoreError::Core)
    }

    fn set_volume(&mut self, volume: f32) {
        self.audio.set_volume(volume);
    }

    fn mapper_save(&self) -> Result<Option<Vec<u8>>, CoreError> {
        let core = self.core_ref()?;
        core.export_mapper_save().map_err(CoreError::Core)
    }

    fn import_mapper_save(&mut self, data: &[u8]) -> Result<(), CoreError> {
        let core = self.core_mut()?;
        core.import_mapper_save(data).map_err(CoreError::Core)
    }

    fn identity(&self) -> Result<SystemIdentity, CoreError> {
        self.core_ref()?
            .rom_identity()
            .into_system_identity()
            .map_err(|e| CoreError::Core(Box::new(e)))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Mutex};

    use crate::input_types::NesInputBuffer;
    use nerust_core_traits::CoreConfig;
    use nerust_input_traits::EmuInput;
    use nerust_render_base::PixelFormat;

    use super::*;
    use crate::{OpenBusReadResult, controller::Controller};

    fn test_emu_input() -> EmuInput {
        use nerust_input_traits::InputStateBuffer;
        let shared: Arc<Mutex<Box<dyn InputStateBuffer>>> =
            Arc::new(Mutex::new(Box::<NesInputBuffer>::default()));
        EmuInput {
            shared,
            flag: Arc::new(AtomicBool::new(false)),
            read_buf: Box::<NesInputBuffer>::default(),
        }
    }

    #[derive(Debug)]
    struct MockController;
    impl Controller for MockController {
        fn read(&mut self, _port: &dyn Port) -> OpenBusReadResult {
            OpenBusReadResult::new(0, 0)
        }
        fn write(&mut self, _port: &dyn Port, _value: u8) {}
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
            ControllerCollection::new(vec![Box::new(MockController)]),
            Box::new(nerust_core_traits::audio::NullAudio),
            test_emu_input(),
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
            ControllerCollection::new(vec![Box::new(MockController)]),
            Box::new(nerust_core_traits::audio::NullAudio),
            test_emu_input(),
        );
        let config = CoreConfig {
            region: None,
            bios_paths: HashMap::new(),
            controllers: HashMap::new(),
            core_options: Vec::new(),
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

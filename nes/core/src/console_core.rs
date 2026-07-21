use std::{
    cell::{Ref, RefCell, RefMut},
    rc::Rc,
};

use nerust_core_traits::{
    ConsoleCore, CoreCapabilities, CoreConfig, CoreError, DynCoreOptionsExt, VideoSignalKind,
    audio::AudioBackend,
    debugger::Debugger,
    identity::SystemIdentity,
};
use nerust_input_traits::{ControllerCollection, ControllerHub as _, EmuInput};
use nerust_render_traits::FrameBuffer;

use crate::{
    Core, cartridge_rom::CartridgeData, core_options::CoreOptions,
    debugger::nes::NesDebugger, input_types::NesInputBuffer,
};

pub(crate) struct SendCore(Rc<RefCell<Option<Core>>>);

unsafe impl Send for SendCore {}

impl SendCore {
    pub(crate) fn new(core: Core) -> Self {
        Self(Rc::new(RefCell::new(Some(core))))
    }

    pub(crate) fn new_empty() -> Self {
        Self(Rc::new(RefCell::new(None)))
    }

    pub(crate) fn set(&mut self, core: Core) {
        *self.0.borrow_mut() = Some(core);
    }

    pub(crate) fn clear(&mut self) {
        *self.0.borrow_mut() = None;
    }

    pub(crate) fn borrow(&self) -> Ref<'_, Option<Core>> {
        self.0.borrow()
    }

    pub(crate) fn borrow_mut(&self) -> RefMut<'_, Option<Core>> {
        self.0.borrow_mut()
    }

    pub(crate) fn clone_rc(&self) -> Self {
        Self(Rc::clone(&self.0))
    }
}

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
            core: SendCore::new(core),
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
            core: SendCore::new_empty(),
            controller,
            audio,
            emu_input,
            paused: false,
        }
    }
}

impl NesConsoleCore {
    fn core_ref(&self) -> Result<Ref<'_, Core>, CoreError> {
        let guard = self.core.borrow();
        if guard.is_some() {
            Ok(Ref::map(guard, |o| o.as_ref().unwrap()))
        } else {
            Err(CoreError::NoRomLoaded)
        }
    }

    fn core_mut(&mut self) -> Result<RefMut<'_, Core>, CoreError> {
        let guard = self.core.borrow_mut();
        if guard.is_some() {
            Ok(RefMut::map(guard, |o| o.as_mut().unwrap()))
        } else {
            Err(CoreError::NoRomLoaded)
        }
    }
}

impl ConsoleCore for NesConsoleCore {
    fn capabilities(&self) -> CoreCapabilities {
        CoreCapabilities {
            output_formats: vec![nerust_render_traits::PixelFormat::PaletteIndex {
                palette: Box::new([0u32; 256]),
            }],
            video_signal: VideoSignalKind::Ntsc,
        }
    }

    fn render_frame(&mut self, frame_slot: &mut FrameBuffer) -> Result<(), CoreError> {
        self.emu_input.take();
        let any: &dyn std::any::Any = &*self.emu_input.read_buf;
        if let Some(state) = any.downcast_ref::<NesInputBuffer>() {
            self.controller.sync_input(&state.0);
        }

        let controller = &mut self.controller;
        let audio = self.audio.as_mut();
        let mut guard = self.core.borrow_mut();
        let core = guard.as_mut().ok_or(CoreError::NoRomLoaded)?;
        core.run_frame(frame_slot, controller, audio);
        Ok(())
    }

    fn load(&mut self, rom: &[u8], config: &CoreConfig) -> Result<(), CoreError> {
        let cartridge_data =
            crate::rom_parse::parse_rom(rom).map_err(|e| CoreError::RomParse(Box::new(e)))?;
        let options = if let Some(core_options) = &config.core_options {
            core_options
                .clone()
                .into_inner()
                .map_err(|_| CoreError::InvalidCoreOptions)?
        } else {
            CoreOptions::default()
        };
        let core = Core::new_with_options(cartridge_data, options).map_err(CoreError::Core)?;
        self.core.set(core);
        self.paused = false;
        Ok(())
    }

    fn unload(&mut self) {
        self.core.clear();
        self.paused = false;
    }

    fn reset(&mut self) {
        if let Ok(mut core) = self.core_mut() {
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
        let mut core = self.core_mut()?;
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
        let mut core = self.core_mut()?;
        core.import_mapper_save(data).map_err(CoreError::Core)
    }

    fn identity(&self) -> Result<SystemIdentity, CoreError> {
        self.core_ref()?
            .rom_identity()
            .into_system_identity()
            .map_err(|e| CoreError::Core(Box::new(e)))
    }

    fn render_frame_with_io(
        &mut self,
        frame_slot: &mut FrameBuffer,
        controller: &mut dyn nerust_input_traits::ControllerHub,
        audio: &mut dyn AudioBackend,
    ) -> Result<u64, CoreError> {
        let mut core = self.core_mut()?;
        Ok(core.run_frame(frame_slot, controller, audio))
    }

    fn create_debugger(&mut self) -> Option<Box<dyn Debugger>> {
        Some(Box::new(NesDebugger::new(self.core.clone_rc())))
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex, atomic::AtomicBool},
    };

    use nerust_core_traits::CoreConfig;
    use nerust_input_traits::{Controller, EmuInput, OpenBusReadResult, Port};
    use nerust_render_traits::PixelFormat;

    use super::*;
    use crate::input_types::NesInputBuffer;

    fn test_emu_input() -> EmuInput {
        use nerust_input_traits::InputStateBuffer;
        let shared: Arc<Mutex<Box<dyn InputStateBuffer>>> =
            Arc::new(Mutex::new(Box::<NesInputBuffer>::default()));
        EmuInput::new(
            shared,
            Arc::new(AtomicBool::new(false)),
            Box::new(|| Box::<NesInputBuffer>::default()),
        )
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

        let cartridge = crate::rom_parse::parse_rom(&rom).expect("parse_rom should succeed");
        assert_eq!(cartridge.mapper_type(), 0);

        let mut core = NesConsoleCore::new(
            cartridge,
            ControllerCollection::new(vec![Box::new(MockController)]),
            Box::new(nerust_core_traits::audio::NullAudio),
            test_emu_input(),
        )
        .expect("NesConsoleCore::new should succeed");

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
            core_options: None,
        };

        let result = ConsoleCore::load(&mut core, &rom, &config);
        assert!(result.is_ok(), "load should succeed: {:?}", result);

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

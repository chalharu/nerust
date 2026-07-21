use std::{
    fs,
    path::PathBuf,
    rc::Rc,
    sync::{Arc, Mutex, atomic::AtomicBool},
    time::{SystemTime, UNIX_EPOCH},
};

use nerust_core_traits::{
    ConsoleCore, CoreCapabilities, CoreConfig, CoreError, CoreOptions,
    audio::{AudioBackend, AudioBackendRegistry},
    factory::{
        CoreFactory, FactoryError,
        load::{DynSystemLoadOptions, MediaObject, ResolvedLoadRequest, SystemLoadOptions},
        settings::FactorySettingsView,
    },
    identity::{SystemId, SystemIdentity},
};
use nerust_gui_runtime::settings::{HostBackendCapabilities, HostWindowCapabilities};
use nerust_input_traits::{
    BufferError, ControllerCollection, ControllerProfile, CreateSplitError, GuiInput,
    InputAssignments, InputPorts, InputResources, InputSplit, InputStateBuffer, InputSystemFactory,
    InputValue, SlotInfo,
};
use nerust_render_traits::{
    FrameBuffer, VideoRenderProfile, logical::LogicalSize, physical::PhysicalSize,
};

use super::SessionHandle;
use crate::settings::factory::settings_view;

/// Placeholder load options with no CLI arguments. Used by mock factories in tests.
#[derive(
    Default, Debug, Clone, PartialEq, Eq, clap::Args, serde::Serialize, serde::Deserialize,
)]
pub(crate) struct NoopSystemLoadOptions;

impl SystemLoadOptions for NoopSystemLoadOptions {}

/// Placeholder core options with no fields. Used by mock factories in tests.
#[derive(Default, Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct NoopCoreOptions;

impl CoreOptions for NoopCoreOptions {}

/// Minimal InputStateBuffer for testing.
#[derive(Debug, Default)]
pub(crate) struct TestInputBuffer([u8; 2]);

impl InputStateBuffer for TestInputBuffer {
    fn set(&mut self, _field: usize, _value: InputValue) -> Result<(), BufferError> {
        Ok(())
    }
    fn clear(&mut self) {
        self.0 = [0; 2];
    }
    fn copy_state(&mut self, other: &dyn InputStateBuffer) {
        let any: &dyn std::any::Any = other;
        if let Some(src) = any.downcast_ref::<TestInputBuffer>() {
            self.0 = src.0;
        }
    }
}

pub(crate) struct MockConsoleCore {
    loaded: bool,
    paused: bool,
    identity: Option<SystemIdentity>,
}

impl MockConsoleCore {
    pub(crate) fn new() -> Self {
        Self {
            loaded: false,
            paused: true,
            identity: None,
        }
    }
}

impl ConsoleCore for MockConsoleCore {
    fn capabilities(&self) -> CoreCapabilities {
        CoreCapabilities {
            output_formats: Vec::new(),
            video_signal: nerust_core_traits::VideoSignalKind::Ntsc,
        }
    }
    fn render_frame(&mut self, _frame_slot: &mut FrameBuffer) -> Result<(), CoreError> {
        Ok(())
    }

    fn load(&mut self, rom: &[u8], _config: &CoreConfig) -> Result<(), CoreError> {
        self.loaded = true;
        self.paused = true;
        self.identity = Some(SystemIdentity::new(
            SystemId::new("nes"),
            rom.get(6..8).unwrap_or(&[0, 0]).to_vec(),
        ));
        Ok(())
    }
    fn unload(&mut self) {
        self.loaded = false;
    }
    fn reset(&mut self) {}
    fn paused(&self) -> bool {
        self.paused
    }
    fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }
    fn save_state(&self) -> Result<Vec<u8>, CoreError> {
        Ok(vec![])
    }
    fn load_state(&mut self, _data: &[u8]) -> Result<(), CoreError> {
        Ok(())
    }
    fn identity(&self) -> Result<SystemIdentity, CoreError> {
        self.identity.clone().ok_or(CoreError::NoRomLoaded)
    }
}

pub(crate) fn test_input_resources() -> (GuiInput, InputSplit) {
    let shared: Arc<Mutex<Box<dyn InputStateBuffer>>> =
        Arc::new(Mutex::new(Box::<TestInputBuffer>::default()));
    let flag = Arc::new(AtomicBool::new(false));
    let gui = GuiInput::new(
        Arc::clone(&shared),
        Arc::clone(&flag),
        Box::new(|| Box::<TestInputBuffer>::default()),
    );
    let split = InputSplit {
        shared: Arc::clone(&shared),
        flag: Arc::clone(&flag),
        new_buffer: Box::new(|| Box::<TestInputBuffer>::default()),
    };
    (gui, split)
}

pub(crate) fn build_test_core_parts() -> nerust_core_traits::factory::CoreParts {
    use nerust_core_traits::factory::CoreParts;
    let core = MockConsoleCore::new();
    let render_profile = VideoRenderProfile {
        source_logical_size: LogicalSize {
            width: 256,
            height: 240,
        },
        logical_size: LogicalSize {
            width: 256,
            height: 240,
        },
        physical_size: PhysicalSize {
            width: 256.0,
            height: 240.0,
        },
        frame_format: nerust_render_traits::VideoFrameFormat::Palette,
        ntsc_packed_rgba8: None,
    };
    let (gui_input, _input_split) = test_input_resources();
    CoreParts {
        core: Box::new(core),
        gui_input,
        field_map: std::collections::HashMap::new(),
        render_profile,
        palette: Box::new([0u32; 256]),
    }
}

#[derive(Debug)]
pub(crate) struct MockInputFactory;
impl InputPorts for MockInputFactory {
    fn slots(&self) -> &[SlotInfo] {
        &[]
    }
    fn controllers(&self) -> Vec<Rc<dyn ControllerProfile>> {
        vec![]
    }
}
impl InputSystemFactory for MockInputFactory {
    fn default_assignments(&self) -> InputAssignments {
        InputAssignments { slots: vec![] }
    }
    fn create_split(&self, _: &ControllerCollection) -> Result<InputResources, CreateSplitError> {
        let shared: Arc<Mutex<Box<dyn InputStateBuffer>>> =
            Arc::new(Mutex::new(Box::<TestInputBuffer>::default()));
        let flag = Arc::new(AtomicBool::new(false));
        Ok(InputResources {
            split: InputSplit {
                shared: Arc::clone(&shared),
                flag: Arc::clone(&flag),
                new_buffer: Box::new(|| Box::<TestInputBuffer>::default()),
            },
            field_map: std::collections::HashMap::new(),
        })
    }
}

pub(crate) struct MockFactory;
impl CoreFactory for MockFactory {
    fn system_id(&self) -> SystemId {
        SystemId::new("nes")
    }

    fn display_name(&self) -> &'static str {
        "NES (test)"
    }

    fn create_core_and_adapter_with_assignments(
        &self,
        _: &nerust_core_traits::factory::settings::FactorySettingsView,
        _speaker: Box<dyn AudioBackend>,
        _: &InputAssignments,
    ) -> Result<nerust_core_traits::factory::CoreParts, FactoryError> {
        Ok(build_test_core_parts())
    }
    fn probe_media(&self, _: &MediaObject) -> bool {
        true
    }
    fn settings_page(
        &self,
        _: &nerust_core_traits::factory::settings::FactorySettingsView,
    ) -> nerust_core_traits::factory::descriptor::SystemSettingsPageModel {
        nerust_core_traits::factory::descriptor::SystemSettingsPageModel {
            fields: Arc::from([]),
        }
    }
    fn apply_settings_choice(
        &self,
        _: &mut nerust_core_traits::factory::settings::FactorySettingsView,
        _: &nerust_core_traits::factory::descriptor::SystemSettingsFieldId,
        _: &nerust_core_traits::factory::descriptor::SystemSettingsChoiceId,
    ) -> Result<(), FactoryError> {
        Ok(())
    }
    fn resolve_load_request(
        &self,
        _: &nerust_core_traits::factory::settings::FactorySettingsView,
        _: Box<dyn DynSystemLoadOptions>,
    ) -> Result<nerust_core_traits::factory::load::ResolvedLoadRequest, FactoryError> {
        Ok(ResolvedLoadRequest {
            options: NoopCoreOptions::default().into(),
        })
    }
    fn default_load_options(&self) -> Box<dyn DynSystemLoadOptions> {
        NoopSystemLoadOptions::default().into()
    }
    fn input_system_factory(&self) -> &dyn InputSystemFactory {
        static MOCK_INPUT: MockInputFactory = MockInputFactory;
        &MOCK_INPUT
    }
}

pub(crate) fn test_session() -> SessionHandle {
    let capabilities = HostBackendCapabilities {
        window: HostWindowCapabilities {
            remembers_window_size: false,
            supports_fullscreen_default: true,
            supports_scaling: true,
        },
        presentation: None,
    };
    let factory: Arc<dyn CoreFactory> = Arc::new(MockFactory);
    let audio_registry = Arc::new(AudioBackendRegistry::new());
    SessionHandle::new_ephemeral(capabilities, factory, audio_registry)
}

pub(crate) fn test_rom() -> Vec<u8> {
    let mut data = vec![0x4E, 0x45, 0x53, 0x1A, 2u8, 1, 0, 0];
    data.resize(16 + 0x8000 + 0x2000, 0);
    data
}

pub(crate) fn test_rom_with_mapper4() -> Vec<u8> {
    let mut data = vec![0x4E, 0x45, 0x53, 0x1A, 2u8, 1, 0x40, 0];
    data.resize(16 + 0x8000 + 0x2000, 0);
    data
}

pub(crate) fn test_view(session: &SessionHandle) -> FactorySettingsView {
    let system_id = session.factory().system_id();
    settings_view(session.settings_snapshot(), &system_id)
}

pub(crate) fn unique_temp_dir(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("current time should be after unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("nerust-{label}-{}-{nonce}", std::process::id()));
    fs::create_dir_all(&path).expect("temp dir should create");
    path
}

use std::{fmt, rc::Rc, sync::Arc};

use nerust_core_traits::{audio::AudioBackendRegistry, factory::CoreFactory};
use nerust_render_base::renderer::GpuFactory;

use crate::load::RomLoader;

pub struct FrontendContext {
    pub gpu_factory: Rc<dyn GpuFactory>,
    pub core_factory: Arc<dyn CoreFactory>,
    pub rom_loader: Box<dyn RomLoader>,
    pub audio_registry: Arc<AudioBackendRegistry>,
}

impl fmt::Debug for FrontendContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FrontendContext")
            .field("gpu_factory", &self.gpu_factory)
            .field("core_factory", &"..")
            .field("rom_loader", &"..")
            .finish()
    }
}

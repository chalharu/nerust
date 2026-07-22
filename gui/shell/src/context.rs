use std::{fmt, rc::Rc, sync::Arc};

use nerust_core_traits::audio::AudioBackendRegistry;
use nerust_render_traits::renderer::GpuFactory;

use crate::{load::RomLoader, registry::SystemRegistry};

pub struct FrontendContext {
    pub gpu_factory: Rc<dyn GpuFactory>,
    pub registry: Arc<SystemRegistry>,
    pub rom_loader: Box<dyn RomLoader>,
    pub audio_registry: Arc<AudioBackendRegistry>,
}

impl fmt::Debug for FrontendContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FrontendContext")
            .field("gpu_factory", &self.gpu_factory)
            .field("systems", &self.registry.all().len())
            .field("rom_loader", &"..")
            .finish()
    }
}

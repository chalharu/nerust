use std::{fmt, rc::Rc, sync::Arc};

use nerust_gui_runtime::settings::HostBackendIdentity;
use nerust_screen_video::GpuFactory;

use crate::factory::CoreFactory;
use crate::load::RomLoader;

pub struct FrontendContext {
    pub gpu_factory: Rc<dyn GpuFactory>,
    pub core_factory: Arc<dyn CoreFactory>,
    pub rom_loader: Box<dyn RomLoader>,
    pub host_backend_identity: HostBackendIdentity,
}

impl fmt::Debug for FrontendContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FrontendContext")
            .field("gpu_factory", &self.gpu_factory)
            .field("core_factory", &"..")
            .field("rom_loader", &"..")
            .field("host_backend_identity", &self.host_backend_identity)
            .finish()
    }
}

use std::{rc::Rc, sync::Arc};

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

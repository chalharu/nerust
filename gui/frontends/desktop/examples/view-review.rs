fn main() {
    use std::{collections::HashMap, rc::Rc, sync::Arc};
    let registry = Arc::new(nerust_gui_shell::registry::SystemRegistry::new(vec![
        Arc::new(nerust_nes_factory::NesFactory),
    ]));
    let context = nerust_gui_shell::context::FrontendContext {
        gpu_factory: Rc::new(nerust_render_softbuffer::SoftbufferFactory),
        rom_loader: registry
            .create_loader(HashMap::new())
            .expect("registered NES loader"),
        registry,
        audio_registry: Arc::new(nerust_core_traits::audio::AudioBackendRegistry::default()),
    };
    if let Err(error) = nerust_desktop::review(context, std::env::args_os().nth(1).map(Into::into))
    {
        eprintln!("View review failed: {error}");
        std::process::exit(1);
    }
}

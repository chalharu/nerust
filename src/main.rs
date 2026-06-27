use nerust_screen_video::GpuFactory;

fn create_factory() -> Box<dyn GpuFactory> {
    #[cfg(feature = "wgpu")]
    return Box::new(nerust_backend_wgpu::WgpuFactory);
    #[cfg(feature = "opengl")]
    return Box::new(nerust_backend_opengl::GlFactory);
}

fn main() {
    #[cfg(feature = "gtk")]
    nerust_gtk::run(create_factory());
    #[cfg(feature = "tao")]
    nerust_tao::run(create_factory());
}

#[cfg(not(any(feature = "wgpu", feature = "opengl")))]
compile_error!("No backend selected. Enable feature 'wgpu' or 'opengl'.");
#[cfg(not(any(feature = "gtk", feature = "tao")))]
compile_error!("No frontend selected. Enable feature 'gtk' or 'tao'.");

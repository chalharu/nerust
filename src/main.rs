use nerust_screen_video::GpuFactory;

fn create_factory() -> Box<dyn GpuFactory> {
    #[cfg(feature = "wgpu")]
    return Box::new(nerust_backend_wgpu::WgpuFactory);
    #[cfg(feature = "opengl")]
    return Box::new(nerust_backend_opengl::GlFactory);
}

#[cfg(feature = "gtk")]
fn main() {
    nerust_gtk::run(create_factory());
}

#[cfg(feature = "tao")]
fn main() {
    nerust_tao::run(create_factory());
}

#[cfg(not(any(feature = "gtk", feature = "tao")))]
fn main() {
    panic!("No frontend selected. Enable feature 'gtk' or 'tao'.");
}

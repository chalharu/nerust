pub use nerust_render_base::SurfaceSize;
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};
use wgpu::{Instance, Surface};

#[cfg(all(target_os = "android", target_arch = "x86_64"))]
pub fn default_instance() -> Instance {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = wgpu::Backends::GL;
    Instance::new(descriptor)
}

#[cfg(not(all(target_os = "android", target_arch = "x86_64")))]
pub fn default_instance() -> Instance {
    Instance::default()
}

pub fn create_wgpu_surface(
    instance: &Instance,
    raw_window_handle: RawWindowHandle,
    raw_display_handle: RawDisplayHandle,
) -> Result<Surface<'static>, String> {
    unsafe {
        instance
            .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: Some(raw_display_handle),
                raw_window_handle,
            })
            .map_err(|err| format!("failed to create wgpu surface: {err:?}"))
    }
}

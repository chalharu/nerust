use raw_window_handle::{RawDisplayHandle, RawWindowHandle};
use wgpu::{Instance, Surface};

fn instance_descriptor() -> wgpu::InstanceDescriptor {
    #[allow(unused_mut)]
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    #[cfg(all(target_os = "android", target_arch = "x86_64"))]
    {
        descriptor.backends = wgpu::Backends::GL;
    }
    descriptor
}

pub fn default_instance() -> Instance {
    Instance::new(instance_descriptor())
}

/// GLES needs the display at instance creation. Keep a genuine native owner,
/// not an unsafe Send/Sync wrapper around a borrowed raw pointer.
pub fn instance_with_display(
    display: impl raw_window_handle::HasDisplayHandle + std::fmt::Debug + Send + Sync + 'static,
) -> Instance {
    Instance::new(instance_descriptor().with_display_handle(Box::new(display)))
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

use raw_window_handle::{HandleError, RawDisplayHandle, RawWindowHandle};
use wgpu::{Instance, Surface};

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct SurfaceSize {
    pub width: u32,
    pub height: u32,
}

impl SurfaceSize {
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }
}

/// # Safety
///
/// Implementors must ensure the returned raw display/window handles always describe the same
/// native surface target, and that the backing native objects outlive any `RenderSurface` built
/// from them.
pub unsafe trait SurfaceTargetSource {
    fn prepare(&self);

    fn surface_size(&self, fallback: SurfaceSize) -> SurfaceSize;

    fn raw_window_handle(&self) -> Result<RawWindowHandle, HandleError>;

    fn raw_display_handle(&self) -> Result<Option<RawDisplayHandle>, HandleError>;
}

pub struct RenderSurface<T> {
    // The surface must drop before the target handles and their backing instance.
    surface: Surface<'static>,
    target: T,
    instance: Instance,
}

impl<T: SurfaceTargetSource> RenderSurface<T> {
    pub fn new(target: T) -> Result<Self, String> {
        let instance = default_instance();
        let surface = create_surface(&instance, &target)?;
        Ok(Self {
            surface,
            target,
            instance,
        })
    }

    pub fn surface_size(&self, fallback: SurfaceSize) -> SurfaceSize {
        self.target.surface_size(fallback)
    }

    pub fn recreate_surface(&mut self) -> Result<(), String> {
        self.surface = create_surface(&self.instance, &self.target)?;
        Ok(())
    }

    pub fn surface(&self) -> &Surface<'static> {
        &self.surface
    }

    pub fn instance(&self) -> &Instance {
        &self.instance
    }
}

#[cfg(all(target_os = "android", target_arch = "x86_64"))]
fn default_instance() -> Instance {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = wgpu::Backends::GL;
    Instance::new(descriptor)
}

#[cfg(not(all(target_os = "android", target_arch = "x86_64")))]
fn default_instance() -> Instance {
    Instance::default()
}

fn create_surface<T: SurfaceTargetSource>(
    instance: &Instance,
    target: &T,
) -> Result<Surface<'static>, String> {
    target.prepare();
    let raw_display_handle = target
        .raw_display_handle()
        .map_err(|err| format!("failed to acquire raw display handle: {err:?}"))?;
    let raw_window_handle = target
        .raw_window_handle()
        .map_err(|err| format!("failed to acquire raw window handle: {err:?}"))?;
    unsafe {
        instance
            .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle,
                raw_window_handle,
            })
            .map_err(|err| format!("failed to create wgpu surface: {err:?}"))
    }
}

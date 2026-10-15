//! Headless GPU factory: always fails closed, never touches hardware.
//! Production code never touches this module.

use nerust_render_traits::renderer::{GpuRenderer, RendererConfig, RendererError};

/// `GpuFactory` that never succeeds: host tests run windowless, so
/// renderer creation must fail closed, never touch the GPU.
#[derive(Debug, Default)]
pub(crate) struct FakeGpuFactory;

impl nerust_render_traits::renderer::GpuFactory for FakeGpuFactory {
    fn create_renderer(
        &self,
        _config: &RendererConfig,
        _display_handle: raw_window_handle::RawDisplayHandle,
    ) -> Result<Box<dyn GpuRenderer>, RendererError> {
        Err(RendererError::new(
            "headless",
            Box::new(std::io::Error::other("no GPU in tests")),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nerust_render_traits::{
        logical::LogicalSize, physical::PhysicalSize, renderer::GpuFactory,
    };

    fn test_config() -> nerust_render_traits::renderer::RendererConfig {
        use nerust_render_traits::{VideoFrameFormat, VideoRenderProfile};
        nerust_render_traits::renderer::RendererConfig {
            render_profile: VideoRenderProfile {
                source_logical_size: LogicalSize {
                    width: 160,
                    height: 144,
                },
                logical_size: LogicalSize {
                    width: 160,
                    height: 144,
                },
                physical_size: PhysicalSize {
                    width: 160.0,
                    height: 144.0,
                },
                frame_format: VideoFrameFormat::Rgba,
                ntsc_packed_rgba8: None,
            },
            vsync: false,
        }
    }
    #[test]
    fn fake_gpu_factory_fails_closed() {
        use raw_window_handle::RawDisplayHandle;
        let factory = FakeGpuFactory;
        // Never dereferenced: the fake fails before touching it.
        let handle: RawDisplayHandle = unsafe { std::mem::zeroed() };
        let result = factory.create_renderer(&test_config(), handle);
        assert!(result.is_err());
    }
}

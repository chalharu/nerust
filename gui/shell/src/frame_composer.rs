use crate::session::osd::{OsdManager, OsdPresenter, OsdSync, OsdSyncInput};
use nerust_render_traits::{FrameBuffer, RenderedImage, SurfaceSize, renderer::RenderPipeline};

pub struct FrameComposer {
    presenter: OsdPresenter,
    raster: nerust_render_osd::OsdRaster,
}

impl Default for FrameComposer {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameComposer {
    pub fn new() -> Self {
        Self {
            presenter: OsdPresenter::new(),
            raster: nerust_render_osd::OsdRaster::new(),
        }
    }
    pub fn invalidate(&mut self) {
        self.presenter.invalidate();
    }
    pub fn render(
        &mut self,
        pipeline: &mut dyn RenderPipeline,
        frame: &FrameBuffer,
        size: SurfaceSize,
        osd: &OsdManager,
        mut sync: OsdSyncInput,
    ) -> Result<Option<RenderedImage>, String> {
        if size.width == 0 || size.height == 0 {
            return Ok(None);
        }
        if pipeline.size() != size {
            pipeline.resize(size);
        }
        sync.width = size.width;
        sync.height = size.height;
        let (top, bottom) = sync.chrome.reserved_rows();
        pipeline.set_reserved_strip(nerust_render_traits::ReservedStrip {
            top: nerust_render_traits::renderer::strip_pixels(size.height, top, sync.ui_scale),
            bottom: nerust_render_traits::renderer::strip_pixels(
                size.height,
                bottom,
                sync.ui_scale,
            ),
        });
        match self.presenter.sync(osd, sync) {
            OsdSync::Skip => {}
            OsdSync::Clear => pipeline.set_osd_layer(None),
            OsdSync::Render(plan) => {
                let (width, height) = (plan.width, plan.height);
                pipeline.set_osd_layer(self.raster.rasterize(&plan.into_content(), width, height));
            }
        }
        let Some(image) = pipeline.render_image(frame, size) else {
            return Ok(None);
        };
        if image.stride_bytes != image.width as usize * 4 {
            return Err("renderer returned a non-packed RGBA frame".into());
        }
        Ok(Some(image))
    }
}

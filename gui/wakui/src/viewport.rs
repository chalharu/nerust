//! Game-viewport abstractions. Widget chrome belongs to native controls;
//! the shared emulation frame belongs here, behind framework traits.
//!
//! Names are fixed in Phase 2; frontend migration follows in Phase 3.
//! Concrete selection (wgpu/gl/softbuffer) is injected by the composition
//! root, never chosen by frontends.

/// Native-side viewport surface (GTK `Picture`, iced texture, ...).
pub trait ViewportSurface {
    /// Cloneable handle shared with the render pipeline.
    type Handle: Clone;
    fn handle(&self) -> Self::Handle;
    fn resize(&mut self, width: u32, height: u32);
}

/// Destination for rendered frames.
pub trait FrameSink {
    type Frame;
    fn present(&mut self, frame: Self::Frame);
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeSurface {
        size: (u32, u32),
        presented: Vec<u32>,
    }

    impl ViewportSurface for FakeSurface {
        type Handle = (u32, u32);
        fn handle(&self) -> Self::Handle {
            self.size
        }
        fn resize(&mut self, width: u32, height: u32) {
            self.size = (width, height);
        }
    }

    impl FrameSink for FakeSurface {
        type Frame = u32;
        fn present(&mut self, frame: Self::Frame) {
            self.presented.push(frame);
        }
    }

    #[test]
    fn surface_and_sink_contract() {
        let mut surface = FakeSurface {
            size: (0, 0),
            presented: Vec::new(),
        };
        surface.resize(256, 240);
        assert_eq!(surface.handle(), (256, 240));
        surface.present(1);
        assert_eq!(surface.presented, vec![1]);
    }
}

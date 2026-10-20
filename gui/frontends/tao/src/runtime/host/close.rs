//! Main-window liveness plus close idempotence in one owned guard.

use std::sync::Arc;
use tao::window::Window as TaoWindow;

/// Single liveness truth. Only `Live` is handle-touchable.
#[derive(Debug, Clone)]
pub(crate) enum MainLiveness {
    Live,
    Closed,
    Failed(Arc<str>),
}

impl MainLiveness {
    /// Creation-failure detail, if terminally failed.
    pub(crate) fn failure(&self) -> Option<&str> {
        match self {
            Self::Failed(detail) => Some(detail),
            _ => None,
        }
    }

    /// Native handle still usable for redraws and surfaces.
    pub(crate) fn is_usable(&self) -> bool {
        matches!(self, Self::Live)
    }
}

/// One-shot close token: `arm` succeeds exactly once per process.
#[derive(Debug, Default)]
pub(crate) struct CloseOnce {
    fired: bool,
}

impl CloseOnce {
    pub(crate) fn new() -> Self {
        Self { fired: false }
    }

    /// Returns true exactly once; later calls are no-ops.
    pub(crate) fn arm(&mut self) -> bool {
        if self.fired {
            return false;
        }
        self.fired = true;
        true
    }

    #[cfg(test)]
    pub(crate) fn fired(&self) -> bool {
        self.fired
    }
}

/// Owned native window handle; all dead-handle checks flow through here.
#[derive(Debug, Clone)]
pub(crate) struct LiveWindow {
    inner: Arc<TaoWindow>,
}

impl LiveWindow {
    pub(crate) fn new(window: Arc<TaoWindow>) -> Self {
        Self { inner: window }
    }

    /// Native redraw request for the owned window.
    pub(crate) fn request_redraw(&self) {
        self.inner.request_redraw();
    }

    /// Current physical size of the owned window.
    pub(crate) fn inner_size(&self) -> tao::dpi::PhysicalSize<u32> {
        self.inner.inner_size()
    }

    /// Native identity for event routing.
    pub(crate) fn id(&self) -> tao::window::WindowId {
        self.inner.id()
    }

    /// Set the window title.
    pub(crate) fn set_title(&self, title: &str) {
        self.inner.set_title(title);
    }

    /// Resize the window (scaling changes).
    pub(crate) fn set_inner_size(&self, size: tao::dpi::LogicalSize<f64>) {
        self.inner.set_inner_size(size);
    }

    /// Current scale factor for logical-size math.
    pub(crate) fn scale_factor(&self) -> f64 {
        self.inner.scale_factor()
    }

    /// Narrow native borrow: surface attach and size queries only.
    pub(crate) fn with_native<R>(&self, f: impl FnOnce(&TaoWindow) -> R) -> R {
        f(&self.inner)
    }

    /// Whether the native window is currently fullscreen.
    pub(crate) fn is_fullscreen(&self) -> bool {
        self.inner.fullscreen().is_some()
    }

    /// Apply a fullscreen transition on the native window.
    pub(crate) fn set_fullscreen(&self, fullscreen: bool) {
        use tao::window::Fullscreen;
        self.inner
            .set_fullscreen(fullscreen.then_some(Fullscreen::Borderless(None)));
    }

    /// Native attach handle plus current surface size, if available.
    pub(crate) fn attach_handle(
        &self,
    ) -> Option<(
        raw_window_handle::RawWindowHandle,
        nerust_render_traits::SurfaceSize,
    )> {
        use nerust_render_traits::SurfaceSize;
        use raw_window_handle::HasWindowHandle;
        let raw = self.inner.window_handle().ok()?.as_raw();
        let size = self.inner.inner_size();
        Some((raw, SurfaceSize::new(size.width, size.height)))
    }
}

/// Fatal creation failure, close idempotence, and native liveness.
/// Query liveness only through [`CloseGuard::is_usable`].
#[derive(Debug)]
pub(crate) struct CloseGuard {
    state: MainLiveness,
    once: CloseOnce,
}

impl CloseGuard {
    pub(crate) fn new() -> Self {
        Self {
            state: MainLiveness::Live,
            once: CloseOnce::new(),
        }
    }

    /// Record a window-creation failure and retire liveness.
    /// `Failed` is terminal: the state stays `Failed`, never `Closed`.
    pub(crate) fn fail(&mut self, detail: String) {
        self.state = MainLiveness::Failed(Arc::from(detail));
    }

    /// Creation failure, if any.
    pub(crate) fn fatal_message(&self) -> Option<&str> {
        self.state.failure()
    }

    /// Native window still usable for redraws and surfaces.
    pub(crate) fn is_usable(&self) -> bool {
        self.state.is_usable()
    }

    /// Idempotent close entry: true exactly once, retiring `Live` to `Closed`.
    pub(crate) fn begin_close(&mut self) -> bool {
        if !self.once.arm() {
            return false;
        }
        if matches!(self.state, MainLiveness::Live) {
            self.state = MainLiveness::Closed;
        }
        true
    }

    /// Close teardown already ran (test read of the guard).
    #[cfg(test)]
    pub(crate) fn exited(&self) -> bool {
        self.once.fired()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn liveness_truth_is_single() {
        assert!(MainLiveness::Live.is_usable());
        assert!(!MainLiveness::Closed.is_usable());
        let failed = MainLiveness::Failed(Arc::from("no display"));
        assert!(!failed.is_usable());
        assert_eq!(failed.failure(), Some("no display"));
        assert_eq!(MainLiveness::Live.failure(), None);
        assert_eq!(MainLiveness::Closed.failure(), None);
    }

    #[test]
    fn close_is_idempotent_and_retires_liveness() {
        let mut guard = CloseGuard::new();
        assert!(guard.is_usable());
        assert!(guard.begin_close());
        assert!(matches!(guard.state, MainLiveness::Closed));
        assert!(!guard.is_usable());
        assert!(!guard.begin_close());
        assert!(guard.exited());
    }

    #[test]
    fn fail_retires_liveness_but_keeps_detail() {
        let mut guard = CloseGuard::new();
        guard.fail("no display".to_string());
        assert_eq!(guard.fatal_message(), Some("no display"));
        assert!(matches!(guard.state, MainLiveness::Failed(_)));
        assert!(!guard.is_usable());
        // A later close still runs exactly once; Failed stays Failed.
        assert!(guard.begin_close());
        assert!(matches!(guard.state, MainLiveness::Failed(_)));
        assert!(!guard.is_usable());
        assert_eq!(guard.fatal_message(), Some("no display"));
    }

    #[test]
    fn close_once_arms_exactly_once() {
        let mut once = CloseOnce::new();
        assert!(once.arm());
        assert!(!once.arm());
        assert!(once.fired());
    }
}

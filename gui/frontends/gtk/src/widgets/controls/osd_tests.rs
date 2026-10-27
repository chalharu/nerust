//! Existing OSD assertions retained after removing the legacy History module.
use nerust_gui_shell::session::osd::{
    OsdChrome, OsdRow, OsdState, OsdSync, OsdSyncInput, TickMetrics, ViewInsets,
};
use nerust_settings_core::model::local::NotificationSettings;

struct StubState {
    generation: u64,
    rows: Vec<OsdRow>,
}
impl OsdState for StubState {
    fn generation(&self) -> u64 {
        self.generation
    }
    fn visible_row(&self) -> Option<&OsdRow> {
        self.rows.first()
    }
    fn rows(&self) -> &[OsdRow] {
        &self.rows
    }
}
fn sync_input(chrome: OsdChrome, width: u32, height: u32) -> OsdSyncInput {
    OsdSyncInput {
        tick: TickMetrics {
            fps: 60.0,
            ..TickMetrics::default()
        },
        chrome,
        language: nerust_settings_core::model::language::AppLanguage::English,
        now: std::time::Instant::now(),
        width,
        height,
        ui_scale: 1.0,
        safe_insets: ViewInsets::default(),
    }
}
fn row_state() -> StubState {
    StubState {
        generation: 7,
        rows: vec![OsdRow {
            text: std::sync::Arc::from("Paused"),
            rgb: (255, 255, 255),
            tag: "[I]".into(),
        }],
    }
}
#[test]
fn osd_new_and_invalidate_are_headless() {
    #[derive(Debug, Default)]
    struct Sink(usize);
    impl nerust_render_traits::renderer::OsdSink for Sink {
        fn set_osd_layer(&mut self, _: Option<nerust_render_traits::renderer::OsdLayer>) {}
    }
    impl nerust_render_traits::renderer::StripSink for Sink {
        fn set_reserved_strip(&mut self, _: nerust_render_traits::ReservedStrip) {
            self.0 += 1;
        }
    }
    let state = crate::window::tests::model_state();
    let ctx =
        nerust_frontend_host::frame::FrameCache::new().frame_ctx(state.borrow().host.session());
    let mut sink = Sink::default();
    let size = nerust_render_traits::SurfaceSize::new(320, 240);
    let now = std::time::Instant::now();
    state
        .borrow_mut()
        .host
        .prepare_osd(size, 240, &ctx, now)
        .apply(&mut sink);
    state
        .borrow_mut()
        .host
        .prepare_osd(size, 240, &ctx, now)
        .apply(&mut sink);
    assert_eq!(sink.0, 1);
    state.borrow_mut().host.invalidate_osd();
    assert!(!state.borrow().host.osd_layer_live());
    state
        .borrow_mut()
        .host
        .prepare_osd(size, 240, &ctx, now)
        .apply(&mut sink);
    assert_eq!(sink.0, 2);
    state.borrow_mut().host.invalidate_osd();
}
#[test]
fn present_plan_renders_then_skips_steady_frames() {
    use nerust_gui_shell::session::osd::OsdPresenter;
    let chrome = OsdChrome::from(&NotificationSettings::default());
    let state = row_state();
    let mut plan = OsdPresenter::new();
    assert!(matches!(
        plan.sync(&state, sync_input(chrome, 320, 240)),
        OsdSync::Render(_)
    ));
    assert!(matches!(
        plan.sync(&state, sync_input(chrome, 320, 240)),
        OsdSync::Skip
    ));
    plan.invalidate();
    assert!(matches!(
        plan.sync(&state, sync_input(chrome, 320, 240)),
        OsdSync::Render(_)
    ));
}
#[test]
fn present_plan_clears_zero_size() {
    use nerust_gui_shell::session::osd::OsdPresenter;
    let chrome = OsdChrome::from(&NotificationSettings::default());
    let idle = StubState {
        generation: 7,
        rows: Vec::new(),
    };
    let mut plan = OsdPresenter::new();
    assert!(matches!(
        plan.sync(&idle, sync_input(chrome, 320, 240)),
        OsdSync::Skip
    ));
    assert!(matches!(
        plan.sync(&idle, sync_input(chrome, 0, 0)),
        OsdSync::Clear | OsdSync::Skip
    ));
}
#[test]
fn raster_backend_rejects_empty_surfaces() {
    use nerust_render_osd::OsdContent;
    use nerust_render_traits::renderer::OsdAnchor;
    let mut raster = nerust_render_osd::OsdRaster::new();
    let content = OsdContent {
        rows: Vec::new(),
        status: None,
        position: OsdAnchor::BottomLeft,
        status_position: OsdAnchor::TopLeft,
    };
    assert!(raster.rasterize(&content, 0, 240).is_none());
    assert!(raster.rasterize(&content, 320, 0).is_none());
}

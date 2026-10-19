//! OSD sync decision: status gate, layer plan, and presenter.
//! Pure decision procedure; frontends only rasterize the plan.

use std::time::{Duration, Instant};

use nerust_render_traits::renderer::{OsdAnchor, SafeInsets};
use nerust_settings_core::model::language::AppLanguage;
use nerust_settings_core::model::local::{
    NotificationAreaMode, NotificationPosition, NotificationSettings,
};

use crate::session::osd_transient::OsdRow;

/// Pause label without the fps value: cheap key for the gate
/// pre-check, so steady frames skip float formatting.
pub(crate) fn status_label(paused: bool, language: AppLanguage) -> &'static str {
    if paused {
        match language {
            AppLanguage::Japanese => "一時停止中",
            _ => "Paused",
        }
    } else {
        ""
    }
}

/// Minimum gap between fps-digit refreshes of the status readout.
/// Pause flips pass through immediately (events); the ticking digits
/// refresh at 2Hz so the readout stays calm and the layer is not
/// re-rasterized every frame on timer jitter.
pub const STATUS_FPS_MIN_INTERVAL: Duration = Duration::from_millis(500);

/// Gates the status value: label changes pass through, fps digits at
/// most every [`STATUS_FPS_MIN_INTERVAL`]. Frontends fold the gated
/// pair into their raster fingerprint alongside the settings.
#[derive(Debug, Default)]
pub struct StatusGate {
    last_label: Option<String>,
    last_value: Option<String>,
    last_tick: Option<Instant>,
}

impl StatusGate {
    pub fn new() -> Self {
        Self::default()
    }

    /// Gated value, computing `fresh` only on a label flip, throttle
    /// expiry, or empty cache. Steady frames skip float formatting.
    pub fn gate_or_cached(
        &mut self,
        now: Instant,
        label: &str,
        fresh: impl FnOnce() -> String,
    ) -> (String, String) {
        let label_changed = self.last_label.as_deref() != Some(label);
        let tick_due = self
            .last_tick
            .is_none_or(|t| now.duration_since(t) >= STATUS_FPS_MIN_INTERVAL);
        if label_changed || tick_due || self.last_value.is_none() {
            let value = fresh();
            self.last_label = Some(label.to_string());
            self.last_value = Some(value.clone());
            self.last_tick = Some(now);
            (label.to_string(), value)
        } else {
            let cached = self.last_value.clone().unwrap_or_else(fresh);
            (label.to_string(), cached)
        }
    }
}

/// Settings corner mapped once to the render-space anchor. The
/// rasterizer only speaks [`OsdAnchor`], so this is the single
/// place where settings vocabulary crosses into render vocabulary.
pub fn osd_anchor(position: NotificationPosition) -> OsdAnchor {
    match position {
        NotificationPosition::TopLeft => OsdAnchor::TopLeft,
        NotificationPosition::TopRight => OsdAnchor::TopRight,
        NotificationPosition::BottomLeft => OsdAnchor::BottomLeft,
        NotificationPosition::BottomRight => OsdAnchor::BottomRight,
    }
}

/// Status pill color. One constant so every frontend renders the
/// persistent readout identically; transient rows keep per-level
/// colors from [`OsdManager::push_all`].
pub const OSD_STATUS_RGB: (u8, u8, u8) = (200, 200, 200);

/// Raster-ready frame decided by [`OsdPresenter`]: rows plus the
/// gated status text, with anchors already mapped to render space.
#[derive(Debug, Clone)]
pub struct OsdPlan {
    pub rows: Vec<OsdRow>,
    pub status: Option<(String, String)>,
    pub position: OsdAnchor,
    pub status_position: OsdAnchor,
    pub width: u32,
    pub height: u32,
}

impl OsdPlan {
    /// Single construction shared by every frontend: rows plus the
    /// status pill with identical colors and corners everywhere.
    /// Takes ownership: no second clone at the renderer boundary.
    pub fn into_content(self) -> nerust_render_traits::renderer::OsdContent {
        use nerust_render_traits::renderer::{OsdContent, OsdRow, OsdStatus};
        OsdContent {
            rows: self
                .rows
                .into_iter()
                .map(|row| OsdRow {
                    tag: row.tag.to_string(),
                    text: row.text,
                    rgb: row.rgb,
                })
                .collect(),
            status: self.status.map(|(label, value)| OsdStatus {
                label,
                value,
                rgb: OSD_STATUS_RGB,
            }),
            position: self.position,
            status_position: self.status_position,
        }
    }
}

/// Narrow chrome view for [`OsdPresenter::sync`]. Only the fields
/// the layer decision reads; a new settings field cannot ripple in.
/// Adapted once per frame at the settings boundary via `From`.
#[derive(Debug, Clone, Copy)]
pub struct OsdChrome {
    pub status_enabled: bool,
    pub position: NotificationPosition,
    pub status_position: NotificationPosition,
    pub area_mode: NotificationAreaMode,
    pub status_area_mode: NotificationAreaMode,
}

impl From<&NotificationSettings> for OsdChrome {
    fn from(settings: &NotificationSettings) -> Self {
        Self {
            status_enabled: settings.status_enabled,
            position: settings.position,
            status_position: settings.status_position,
            area_mode: settings.area_mode,
            status_area_mode: settings.status_area_mode,
        }
    }
}

/// Per-frame inputs for [`OsdPresenter::sync`]. Bundled so the
/// decision signature stays narrow as inputs evolve.
pub struct OsdSyncInput {
    pub chrome: OsdChrome,
    pub language: AppLanguage,
    pub fps: f32,
    pub paused: bool,
    pub now: Instant,
    pub width: u32,
    pub height: u32,
    /// Raster scale and safe insets: they move boxes without
    /// touching settings, so the fingerprint must see them or the
    /// gate would suppress the re-render.
    pub ui_scale: f32,
    pub safe_insets: SafeInsets,
}

/// Per-frame layer decision. `Clear` is only emitted while a layer
/// is live, so frontends call the renderer exactly as told.
#[derive(Debug, Clone)]
pub enum OsdSync {
    Skip,
    Clear,
    Render(OsdPlan),
}

/// Narrow view of display state for [`OsdPresenter::sync`]. The manager
/// implements this; the presenter never names the manager type, so the
/// decision stays decoupled from the orchestration.
pub trait OsdState {
    fn generation(&self) -> u64;
    fn visible_row(&self) -> Option<&OsdRow>;
    fn rows(&self) -> Vec<OsdRow>;
}

/// Owns the whole sync fingerprint (transient dirty flag, status
/// gate, surface size, layer liveness) so tao/gtk/android share one
/// decision procedure instead of reimplementing it per frontend.
/// Frontends only rasterize the [`OsdPlan`] and hand it to the
/// renderer; all policy lives here and is headless-testable.
#[derive(Debug, Default)]
pub struct OsdPresenter {
    last_seen: u64,
    gate: StatusGate,
    last_status: Option<(
        String,
        String,
        NotificationPosition,
        NotificationPosition,
        NotificationAreaMode,
        NotificationAreaMode,
    )>,
    last_size: Option<(u32, u32)>,
    last_chrome: Option<(nerust_render_traits::renderer::SafeInsets, u32)>,
    active: bool,
}

impl OsdPresenter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Forget all sync state so the next pass re-pushes the layer
    /// from scratch. Call after the renderer is replaced: the new
    /// object holds no layer for the gate to skip against.
    pub fn invalidate(&mut self) {
        *self = Self::default();
    }

    /// Last display-generation this presenter synced. The pre-gate
    /// compares it without forcing a full sync.
    pub fn last_seen(&self) -> u64 {
        self.last_seen
    }

    /// Decide the layer action for this frame. The generation check
    /// replaces the old peek-then-take flag: inequality against the
    /// last synced generation means re-render. Status rides outside
    /// that flag: pause flips and settings toggles re-render
    /// immediately, fps digits refresh at 2Hz through the gate. The
    /// gate computes fresh text lazily, so steady frames skip float
    /// formatting. Untouched while disabled: no per-frame formatting,
    /// and the stale gate passes the first fresh value on re-enable.
    pub fn sync(&mut self, state: &impl OsdState, input: OsdSyncInput) -> OsdSync {
        let OsdSyncInput {
            chrome,
            language,
            fps,
            paused,
            now,
            width,
            height,
            ui_scale,
            safe_insets,
        } = input;
        let changed = state.generation() != self.last_seen;
        self.last_seen = state.generation();
        if width == 0 || height == 0 {
            if self.active {
                self.active = false;
                return OsdSync::Clear;
            }
            return OsdSync::Skip;
        }
        let status_key = chrome.status_enabled.then(|| {
            let (label, value) =
                self.gate
                    .gate_or_cached(now, status_label(paused, language), || {
                        format!("{fps:.1} fps")
                    });
            (
                label,
                value,
                chrome.position,
                chrome.status_position,
                chrome.area_mode,
                chrome.status_area_mode,
            )
        });
        if status_key.is_none() {
            // Forget the throttle: re-enable must pass fresh digits.
            self.gate = StatusGate::new();
        }
        let status_changed = status_key != self.last_status;
        let raster = (safe_insets, ui_scale.to_bits());
        let chrome_changed = self.last_chrome != Some(raster);
        if !changed && !status_changed && !chrome_changed && self.last_size == Some((width, height))
        {
            return OsdSync::Skip;
        }
        self.last_size = Some((width, height));
        self.last_chrome = Some(raster);
        self.last_status = status_key.clone();
        let empty = state.visible_row().is_none() && status_key.is_none();
        if empty {
            if self.active {
                self.active = false;
                return OsdSync::Clear;
            }
            return OsdSync::Skip;
        }
        self.active = true;
        let (label_value, position, status_position) = match status_key {
            Some((label, value, position, status_position, _, _)) => {
                (Some((label, value)), position, status_position)
            }
            None => (None, chrome.position, chrome.status_position),
        };
        OsdSync::Render(OsdPlan {
            rows: state.rows(),
            status: label_value,
            position: osd_anchor(position),
            status_position: osd_anchor(status_position),
            width,
            height,
        })
    }
}

/// Strip rows per edge from transient + status corners.
/// Same edge, different sides share one strip; same corner stacks two.
/// Each readout reserves only under its own area mode.
/// When both readouts share one horizontal band (both top or both
/// bottom), the transient area mode governs the edge: the status
/// picker follows it in the UI, so an overlay status never dangles
/// half inside a strip the transient reserved.
pub fn strip_rows(
    transient_mode: NotificationAreaMode,
    transient: NotificationPosition,
    status_mode: NotificationAreaMode,
    status: NotificationPosition,
    status_on: bool,
) -> (u32, u32) {
    let status_mode = if status_on && transient.same_edge(status) {
        transient_mode
    } else {
        status_mode
    };
    fn edge(position: NotificationPosition) -> bool {
        matches!(
            position,
            NotificationPosition::TopLeft | NotificationPosition::TopRight
        )
    }
    let mut top = [0u32, 0u32];
    let mut bottom = [0u32, 0u32];
    let sides = [
        (transient_mode, transient, true),
        (status_mode, status, status_on),
    ];
    for (mode, position, on) in sides {
        if !on || !matches!(mode, NotificationAreaMode::Strip) {
            continue;
        }
        let side = usize::from(matches!(
            position,
            NotificationPosition::TopRight | NotificationPosition::BottomRight
        ));
        if edge(position) {
            top[side] += 1;
        } else {
            bottom[side] += 1;
        }
    }
    (top[0].max(top[1]), bottom[0].max(bottom[1]))
}

#[cfg(test)]
mod tests {
    use super::{
        OsdChrome, OsdPresenter, OsdRow, OsdState, OsdSync, OsdSyncInput, STATUS_FPS_MIN_INTERVAL,
        StatusGate, osd_anchor, strip_rows,
    };
    use nerust_render_traits::renderer::SafeInsets;
    use nerust_settings_core::model::language::AppLanguage;
    use nerust_settings_core::model::local::{
        NotificationAreaMode as M, NotificationPosition as P, NotificationSettings,
    };
    use std::time::{Duration, Instant};

    /// Fixed display source for decision tests. Bumping the generation
    /// stands in for a push or poll that changed the showing row.
    struct StubState {
        generation: u64,
        row: Option<OsdRow>,
    }

    impl OsdState for StubState {
        fn generation(&self) -> u64 {
            self.generation
        }

        fn visible_row(&self) -> Option<&OsdRow> {
            self.row.as_ref()
        }

        fn rows(&self) -> Vec<OsdRow> {
            self.row.clone().into_iter().collect()
        }
    }

    fn idle_state() -> StubState {
        StubState {
            generation: 7,
            row: None,
        }
    }

    fn row_state(text: &str) -> StubState {
        StubState {
            generation: 7,
            row: Some(OsdRow {
                text: text.to_string(),
                rgb: (255, 255, 255),
                tag: "[I]",
            }),
        }
    }

    fn presenter_settings() -> NotificationSettings {
        NotificationSettings::default()
    }

    fn chrome_of(settings: &NotificationSettings) -> OsdChrome {
        OsdChrome::from(settings)
    }

    fn sync_input(
        chrome: OsdChrome,
        now: Instant,
        width: u32,
        height: u32,
        ui_scale: f32,
        safe_insets: SafeInsets,
    ) -> OsdSyncInput {
        OsdSyncInput {
            chrome,
            language: AppLanguage::English,
            fps: 60.0,
            paused: false,
            now,
            width,
            height,
            ui_scale,
            safe_insets,
        }
    }

    #[test]
    fn status_gate_throttles_digits_but_not_labels() {
        let mut gate = StatusGate::new();
        let now = Instant::now();
        // First sample passes.
        assert_eq!(
            gate.gate_or_cached(now, "", || "60.1 fps".to_string()),
            (String::new(), "60.1 fps".to_string())
        );
        // Jitter within the interval holds the old digits.
        assert_eq!(
            gate.gate_or_cached(now, "", || "60.2 fps".to_string()),
            (String::new(), "60.1 fps".to_string())
        );
        // After the interval the fresh digits pass.
        assert_eq!(
            gate.gate_or_cached(now + STATUS_FPS_MIN_INTERVAL, "", || {
                "60.2 fps".to_string()
            }),
            (String::new(), "60.2 fps".to_string())
        );
        // A pause flip passes immediately even inside the interval.
        assert_eq!(
            gate.gate_or_cached(now + STATUS_FPS_MIN_INTERVAL, "Paused", || {
                "60.3 fps".to_string()
            }),
            ("Paused".to_string(), "60.3 fps".to_string())
        );
    }

    #[test]
    fn gate_or_cached_computes_fresh_only_on_change() {
        let mut gate = StatusGate::new();
        let now = Instant::now();
        // Fresh gate: fresh computed once.
        let mut fresh_calls = 0;
        let (label, value) = gate.gate_or_cached(now, "Paused", || {
            fresh_calls += 1;
            "60.0 fps".to_string()
        });
        assert_eq!((label.as_str(), value.as_str()), ("Paused", "60.0 fps"));
        assert_eq!(fresh_calls, 1);
        // Steady: fresh never runs again.
        let (_, steady) = gate.gate_or_cached(now, "Paused", || {
            fresh_calls += 1;
            "60.1 fps".to_string()
        });
        assert_eq!(steady, "60.0 fps");
        assert_eq!(fresh_calls, 1);
    }

    #[test]
    fn strip_rows_share_edges() {
        use nerust_settings_core::model::local::{
            NotificationAreaMode as Mode, NotificationPosition as Pos,
        };
        assert_eq!(
            strip_rows(
                Mode::Strip,
                Pos::BottomLeft,
                Mode::Strip,
                Pos::BottomRight,
                true
            ),
            (0, 1)
        );
        assert_eq!(
            strip_rows(
                Mode::Strip,
                Pos::BottomLeft,
                Mode::Strip,
                Pos::BottomLeft,
                true
            ),
            (0, 2)
        );
        assert_eq!(
            strip_rows(
                Mode::Overlay,
                Pos::BottomLeft,
                Mode::Overlay,
                Pos::TopLeft,
                true
            ),
            (0, 0)
        );
        // Independent modes: overlay status reserves nothing.
        assert_eq!(
            strip_rows(
                Mode::Strip,
                Pos::BottomLeft,
                Mode::Overlay,
                Pos::TopLeft,
                true
            ),
            (0, 1)
        );
        // ... and an overlay transient leaves a strip status alone.
        assert_eq!(
            strip_rows(
                Mode::Overlay,
                Pos::BottomLeft,
                Mode::Strip,
                Pos::TopLeft,
                true
            ),
            (1, 0)
        );
    }

    #[test]
    fn strip_rows_counts_edges_and_sides() {
        assert_eq!(
            strip_rows(M::Overlay, P::TopLeft, M::Overlay, P::BottomLeft, true),
            (0, 0)
        );
        assert_eq!(
            strip_rows(M::Strip, P::TopLeft, M::Strip, P::BottomLeft, true),
            (1, 1)
        );
        assert_eq!(
            strip_rows(M::Strip, P::TopRight, M::Strip, P::BottomRight, true),
            (1, 1)
        );
        // Same corner stacks two rows on one side.
        assert_eq!(
            strip_rows(M::Strip, P::TopLeft, M::Strip, P::TopLeft, true),
            (2, 0)
        );
        // Status off: transient row only.
        assert_eq!(
            strip_rows(M::Strip, P::BottomLeft, M::Strip, P::TopLeft, false),
            (0, 1)
        );
        // Same band: the transient area mode governs the edge, so the
        // status picker cannot dangle an overlay inside a strip.
        assert_eq!(
            strip_rows(M::Strip, P::BottomLeft, M::Overlay, P::BottomLeft, true),
            (0, 2)
        );
        assert_eq!(
            strip_rows(M::Overlay, P::BottomLeft, M::Strip, P::BottomLeft, true),
            (0, 0)
        );
        assert_eq!(
            strip_rows(M::Strip, P::BottomLeft, M::Overlay, P::BottomRight, true),
            (0, 1)
        );
        assert_eq!(
            strip_rows(M::Overlay, P::BottomLeft, M::Strip, P::BottomRight, true),
            (0, 0)
        );
    }

    #[test]
    fn anchor_mapping_covers_all_corners() {
        use nerust_render_traits::renderer::OsdAnchor as A;
        assert_eq!(osd_anchor(P::TopLeft), A::TopLeft);
        assert_eq!(osd_anchor(P::TopRight), A::TopRight);
        assert_eq!(osd_anchor(P::BottomLeft), A::BottomLeft);
        assert_eq!(osd_anchor(P::BottomRight), A::BottomRight);
    }

    #[test]
    fn into_content_moves_rows_without_second_clone() {
        let state = row_state("Paused");
        let mut presenter = OsdPresenter::new();
        let now = Instant::now();
        let plan = match presenter.sync(
            &state,
            sync_input(
                chrome_of(&presenter_settings()),
                now,
                320,
                240,
                1.0,
                SafeInsets::default(),
            ),
        ) {
            OsdSync::Render(plan) => plan,
            other => panic!("expected render, got {other:?}"),
        };
        assert_eq!(plan.rows.len(), 1);
        let content = plan.into_content();
        assert_eq!(content.rows.len(), 1);
        assert_eq!(content.rows[0].text, "Paused");
    }

    #[test]
    fn presenter_skips_when_idle_and_clears_zero_size() {
        let state = idle_state();
        let mut presenter = OsdPresenter::new();
        let settings = presenter_settings();
        // Idle, no status: nothing to draw, nothing live.
        assert!(matches!(
            presenter.sync(
                &state,
                sync_input(
                    chrome_of(&settings),
                    Instant::now(),
                    960,
                    720,
                    1.0,
                    Default::default()
                )
            ),
            OsdSync::Skip
        ));
        // Zero-size while idle stays skipped.
        assert!(matches!(
            presenter.sync(
                &state,
                sync_input(
                    chrome_of(&settings),
                    Instant::now(),
                    0,
                    0,
                    1.0,
                    Default::default()
                )
            ),
            OsdSync::Skip
        ));
    }

    #[test]
    fn presenter_renders_status_then_skips_and_clears() {
        let state = idle_state();
        let mut presenter = OsdPresenter::new();
        let mut settings = presenter_settings();
        settings.status_enabled = true;
        let now = Instant::now();
        // First frame renders the status readout.
        let plan = match presenter.sync(
            &state,
            sync_input(chrome_of(&settings), now, 960, 720, 1.0, Default::default()),
        ) {
            OsdSync::Render(plan) => plan,
            other => panic!("expected render, got {other:?}"),
        };
        assert_eq!(
            plan.status.as_ref().map(|(l, v)| (l.as_str(), v.as_str())),
            Some(("", "60.0 fps"))
        );
        assert_eq!(plan.width, 960);
        // Same inputs: fps jitter inside the gate holds the digits.
        assert!(matches!(
            presenter.sync(
                &state,
                OsdSyncInput {
                    fps: 60.2,
                    ..sync_input(chrome_of(&settings), now, 960, 720, 1.0, Default::default())
                }
            ),
            OsdSync::Skip
        ));
        // Pause flips pass through immediately.
        assert!(matches!(
            presenter.sync(
                &state,
                OsdSyncInput {
                    paused: true,
                    fps: 60.2,
                    ..sync_input(chrome_of(&settings), now, 960, 720, 1.0, Default::default())
                }
            ),
            OsdSync::Render(_)
        ));
        // Disabling status with no rows clears the live layer.
        settings.status_enabled = false;
        assert!(matches!(
            presenter.sync(
                &state,
                sync_input(chrome_of(&settings), now, 960, 720, 1.0, Default::default()),
            ),
            OsdSync::Clear
        ));
        // Clearing again stays skipped: nothing live.
        assert!(matches!(
            presenter.sync(
                &state,
                sync_input(chrome_of(&settings), now, 960, 720, 1.0, Default::default()),
            ),
            OsdSync::Skip
        ));
    }

    #[test]
    fn presenter_rerenders_on_chrome_change() {
        let state = idle_state();
        let mut presenter = OsdPresenter::new();
        let mut settings = presenter_settings();
        settings.status_enabled = true;
        let now = Instant::now();
        let plain = SafeInsets::default();
        let inset = SafeInsets {
            top: 40,
            right: 24,
            bottom: 40,
            left: 24,
        };
        // First frame renders the status readout.
        assert!(matches!(
            presenter.sync(
                &state,
                sync_input(chrome_of(&settings), now, 960, 720, 1.0, plain)
            ),
            OsdSync::Render(_)
        ));
        // Steady inputs skip.
        assert!(matches!(
            presenter.sync(
                &state,
                sync_input(chrome_of(&settings), now, 960, 720, 1.0, plain)
            ),
            OsdSync::Skip
        ));
        // Insets move the boxes: re-render without settings churn.
        assert!(matches!(
            presenter.sync(
                &state,
                sync_input(chrome_of(&settings), now, 960, 720, 1.0, inset)
            ),
            OsdSync::Render(_)
        ));
        // Scale change re-renders too.
        assert!(matches!(
            presenter.sync(
                &state,
                sync_input(chrome_of(&settings), now, 960, 720, 0.5, inset)
            ),
            OsdSync::Render(_)
        ));
        // Steady again skips.
        assert!(matches!(
            presenter.sync(
                &state,
                sync_input(chrome_of(&settings), now, 960, 720, 0.5, inset)
            ),
            OsdSync::Skip
        ));
    }

    #[test]
    fn presenter_invalidate_forces_full_repush() {
        let state = idle_state();
        let mut presenter = OsdPresenter::new();
        let mut settings = presenter_settings();
        settings.status_enabled = true;
        let now = Instant::now();
        let input = || {
            sync_input(
                chrome_of(&settings),
                now,
                960,
                720,
                1.0,
                SafeInsets::default(),
            )
        };
        // First frame renders the status readout, the repeat skips.
        assert!(matches!(
            presenter.sync(&state, input()),
            OsdSync::Render(_)
        ));
        assert!(matches!(presenter.sync(&state, input()), OsdSync::Skip));
        // Invalidate forgets the fingerprint: the next pass re-pushes.
        presenter.invalidate();
        assert!(matches!(
            presenter.sync(&state, input()),
            OsdSync::Render(_)
        ));
    }

    #[test]
    fn stable_generation_skips_until_bump() {
        let state = row_state("Paused");
        let mut presenter = OsdPresenter::new();
        let settings = presenter_settings();
        let now = Instant::now();
        let input = || sync_input(chrome_of(&settings), now, 960, 720, 1.0, Default::default());
        // First sighting renders; repeats skip without starving later work.
        assert!(matches!(
            presenter.sync(&state, input()),
            OsdSync::Render(_)
        ));
        assert!(matches!(presenter.sync(&state, input()), OsdSync::Skip));
        assert!(matches!(presenter.sync(&state, input()), OsdSync::Skip));
        // A generation bump re-renders even with identical rows.
        let bumped = StubState {
            generation: state.generation + 1,
            row: state.row.clone(),
        };
        assert!(matches!(
            presenter.sync(&bumped, input()),
            OsdSync::Render(_)
        ));
        assert!(matches!(presenter.sync(&bumped, input()), OsdSync::Skip));
    }

    #[test]
    fn row_clear_settles_from_clear_to_skip() {
        let mut presenter = OsdPresenter::new();
        let settings = presenter_settings();
        let now = Instant::now();
        let input = || sync_input(chrome_of(&settings), now, 960, 720, 1.0, Default::default());
        assert!(matches!(
            presenter.sync(&row_state("Paused"), input()),
            OsdSync::Render(_)
        ));
        // Row gone with a generation bump: clear once, then settle.
        let cleared = StubState {
            generation: 8,
            row: None,
        };
        assert!(matches!(presenter.sync(&cleared, input()), OsdSync::Clear));
        assert!(matches!(presenter.sync(&cleared, input()), OsdSync::Skip));
    }

    #[test]
    fn status_reenable_passes_fresh_value() {
        let state = idle_state();
        let mut presenter = OsdPresenter::new();
        let mut settings = presenter_settings();
        settings.status_enabled = true;
        let now = Instant::now();
        assert!(matches!(
            presenter.sync(
                &state,
                OsdSyncInput {
                    fps: 60.0,
                    ..sync_input(chrome_of(&settings), now, 960, 720, 1.0, Default::default())
                }
            ),
            OsdSync::Render(_)
        ));
        // Disabled: the live layer clears, then idles.
        settings.status_enabled = false;
        let off = now + Duration::from_millis(100);
        assert!(matches!(
            presenter.sync(
                &state,
                OsdSyncInput {
                    fps: 61.0,
                    now: off,
                    ..sync_input(chrome_of(&settings), now, 960, 720, 1.0, Default::default())
                }
            ),
            OsdSync::Clear
        ));
        // Re-enabled within the throttle window: fresh digits, not stale.
        settings.status_enabled = true;
        let plan = match presenter.sync(
            &state,
            OsdSyncInput {
                fps: 61.0,
                now: off,
                ..sync_input(chrome_of(&settings), now, 960, 720, 1.0, Default::default())
            },
        ) {
            OsdSync::Render(plan) => plan,
            other => panic!("expected render, got {other:?}"),
        };
        assert_eq!(
            plan.status.as_ref().map(|(l, v)| (l.as_str(), v.as_str())),
            Some(("", "61.0 fps"))
        );
    }
}

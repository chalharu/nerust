//! OSD sync decision: status gate, layer plan, and presenter.
//! Pure decision procedure; frontends only rasterize the plan.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

pub use nerust_render_traits::renderer::ViewInsets;
use nerust_settings_core::model::language::AppLanguage;
use nerust_settings_core::model::local::{
    NotificationAreaMode, NotificationPosition, NotificationSettings,
};

use crate::session::{
    notification::{Message, NotificationLevel},
    osd_transient::{FrameChannel, OsdRow, TransientPolicy, falls_back_to_modal},
};

/// One metrics read per tick: `metrics()` locks and queries the emu thread.
#[derive(Debug, Clone, Copy, Default)]
pub struct TickMetrics {
    pub fps: f32,
    pub paused: bool,
    pub loaded: bool,
    pub frame_counter: u64,
}

impl TickMetrics {
    /// Display channel for notifications: without a loaded core no
    /// frame is composited, so the OSD is invisible.
    pub fn frame_channel(&self) -> FrameChannel {
        if self.loaded {
            FrameChannel::Composited
        } else {
            FrameChannel::Unpresented
        }
    }

    pub fn can_pause(&self) -> bool {
        self.loaded && !self.paused
    }

    pub fn can_resume(&self) -> bool {
        self.loaded && self.paused
    }
}

/// Stable status key for the readout; the sync fingerprint keys on it, not on localized text.
pub(crate) fn status_key(paused: bool) -> &'static str {
    if paused { "paused" } else { "" }
}

/// Display text for a status key. Single translation point: every
/// frontend shows the same wording for the same key and language.
pub(crate) fn status_text(key: &str, language: AppLanguage) -> &'static str {
    match (key, language) {
        ("paused", AppLanguage::Japanese) => "一時停止中",
        ("paused", _) => "Paused",
        _ => "",
    }
}

/// Push-side routing: transient row, modal batch, or drop.
/// The channel-to-modal policy lives here, not in the queue.
pub(crate) enum ModalRoute {
    Transient,
    Modal,
    Drop,
}

/// Narrow routing view: policy plus display channel. Decision code
/// takes this instead of the full settings surface.
pub(crate) struct ModalRouter<'a> {
    pub policy: &'a TransientPolicy,
    pub channel: FrameChannel,
}

impl ModalRouter<'_> {
    pub(crate) fn route(&self, kind: &Message) -> ModalRoute {
        let level: NotificationLevel = kind.level();
        if !self.policy.enabled(level) {
            return ModalRoute::Drop;
        }
        // No composited frame and not modal-worthy: drop the row.
        if self.channel == FrameChannel::Unpresented && !kind.wants_modal() {
            return ModalRoute::Drop;
        }
        if falls_back_to_modal(kind, self.policy, self.channel) {
            ModalRoute::Modal
        } else {
            ModalRoute::Transient
        }
    }
}

/// Minimum gap between fps-digit refreshes of the status readout; pause flips pass through.
pub const STATUS_FPS_MIN_INTERVAL: Duration = Duration::from_millis(500);

/// Single source for the status fps text, shared by the gate and the OSD readout.
pub fn format_status_value(fps: f32) -> String {
    format!("{fps:.1} fps")
}

/// Gates the status value: label changes pass through, fps digits throttled.
/// Frontends fold the gated pair into their raster fingerprint alongside the settings.
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

    /// Gated value, computing `fresh` only on a label flip, throttle expiry, or empty cache.
    /// Borrows the cached pair: callers clone out only on change.
    pub fn gate_or_cached(
        &mut self,
        now: Instant,
        label: &str,
        fresh: impl FnOnce() -> String,
    ) -> (&str, &str) {
        let label_changed = self.last_label.as_deref() != Some(label);
        let tick_due = self
            .last_tick
            .is_none_or(|t| now.duration_since(t) >= STATUS_FPS_MIN_INTERVAL);
        if label_changed || tick_due || self.last_value.is_none() {
            let value = fresh();
            self.last_label = Some(label.to_string());
            self.last_value = Some(value);
            self.last_tick = Some(now);
        }
        (
            self.last_label.as_deref().unwrap_or(""),
            self.last_value.as_deref().unwrap_or(""),
        )
    }
}

/// Raster-ready frame decided by [`OsdPresenter`]: rows plus gated status text.
/// Status shares the fingerprint pair: steady renders clone the `Arc`.
#[derive(Debug, Clone)]
pub struct OsdPlan {
    pub rows: Vec<OsdRow>,
    pub status: Option<(Arc<str>, Arc<str>)>,
    pub position: NotificationPosition,
    pub status_position: NotificationPosition,
    pub width: u32,
    pub height: u32,
}

/// Narrow chrome view for [`OsdPresenter::sync`]: only the fields the layer decision reads.
/// Adapted once per frame at the settings boundary via `From`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OsdChrome {
    pub status_enabled: bool,
    pub position: NotificationPosition,
    pub status_position: NotificationPosition,
    pub area_mode: NotificationAreaMode,
    pub status_area_mode: NotificationAreaMode,
}

impl OsdChrome {
    /// Reserved strip rows per edge for the current chrome; shared by every frontend.
    /// Same edge shares one strip; each readout reserves only under its own area mode.
    pub fn reserved_rows(&self) -> (u32, u32) {
        let status_mode = if self.status_enabled && self.position.same_edge(self.status_position) {
            self.area_mode
        } else {
            self.status_area_mode
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
            (self.area_mode, self.position, true),
            (status_mode, self.status_position, self.status_enabled),
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

/// Per-frame inputs for [`OsdPresenter::sync`], bundled so the decision signature stays narrow.
/// The tick is the single metrics read for the pass: fps and pause ride it.
pub struct OsdSyncInput {
    pub tick: TickMetrics,
    pub chrome: OsdChrome,
    pub language: AppLanguage,
    pub now: Instant,
    pub width: u32,
    pub height: u32,
    /// Raster scale and safe insets: the fingerprint must see them or re-render is suppressed.
    pub ui_scale: f32,
    pub safe_insets: ViewInsets,
}

/// Per-frame layer decision. `Clear` is only emitted while a layer
/// is live, so frontends call the renderer exactly as told.
#[derive(Debug, Clone)]
pub enum OsdSync {
    Skip,
    Clear,
    Render(OsdPlan),
}

/// Narrow view of display state for [`OsdPresenter::sync`], decoupled from the orchestration.
pub trait OsdState {
    fn generation(&self) -> u64;
    fn visible_row(&self) -> Option<&OsdRow>;
    fn rows(&self) -> &[OsdRow];
}

/// Status half of the sync fingerprint; `Arc` shares the pair without per-render clones.
#[derive(Debug, Clone, PartialEq, Eq)]
struct StatusFingerprint {
    label: std::sync::Arc<str>,
    value: std::sync::Arc<str>,
    position: NotificationPosition,
    status_position: NotificationPosition,
    area_mode: NotificationAreaMode,
    status_area_mode: NotificationAreaMode,
}

/// Owns the whole sync fingerprint so all frontends share one decision procedure.
/// Frontends only rasterize the [`OsdPlan`] and hand it to the renderer.
#[derive(Debug, Default)]
pub struct OsdPresenter {
    last_seen: u64,
    gate: StatusGate,
    last_status: Option<StatusFingerprint>,
    last_size: Option<(u32, u32)>,
    /// Last layer geometry; the status tuple exists only while enabled.
    /// A position flip with status off needs its own fingerprint or the Skip gate sticks.
    last_chrome: Option<OsdChrome>,
    last_raster: Option<(ViewInsets, u32)>,
    active: bool,
}

impl OsdPresenter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Forget all sync state so the next pass re-pushes the layer from scratch.
    /// Call after the renderer is replaced: the new object holds no layer to skip against.
    pub fn invalidate(&mut self) {
        *self = Self::default();
    }

    /// Last display-generation this presenter synced. The pre-gate
    /// compares it without forcing a full sync.
    pub fn last_seen(&self) -> u64 {
        self.last_seen
    }

    /// Content gate: generation is compared by the caller; this folds the status readout.
    /// Borrow-compares before allocating, so steady skipped frames never format or box.
    fn sync_status(
        &mut self,
        tick: &TickMetrics,
        chrome: &OsdChrome,
        language: AppLanguage,
        now: Instant,
    ) -> bool {
        let mut status_fingerprint: Option<StatusFingerprint> = None;
        let status_changed = if chrome.status_enabled {
            let key = status_key(tick.paused);
            let label = status_text(key, language);
            let (_, cached_value) = self
                .gate
                .gate_or_cached(now, key, || format_status_value(tick.fps));
            let same = self.last_status.as_ref().is_some_and(|last| {
                last.label.as_ref() == label
                    && last.value.as_ref() == cached_value
                    && last.position == chrome.position
                    && last.status_position == chrome.status_position
                    && last.area_mode == chrome.area_mode
                    && last.status_area_mode == chrome.status_area_mode
            });
            let changed_now = !same;
            if changed_now {
                status_fingerprint = Some(StatusFingerprint {
                    label: label.into(),
                    value: cached_value.into(),
                    position: chrome.position,
                    status_position: chrome.status_position,
                    area_mode: chrome.area_mode,
                    status_area_mode: chrome.status_area_mode,
                });
            }
            changed_now
        } else {
            // Forget the throttle: re-enable must pass fresh digits.
            self.gate = StatusGate::new();
            self.last_status.is_some()
        };
        if status_changed {
            // Only a real change replaces the stored fingerprint; the
            // unchanged path keeps serving it without cloning.
            self.last_status = status_fingerprint;
        }
        status_changed
    }

    /// Geometry gate: chrome, raster, and size only invalidate the cached plan.
    fn sync_geometry(
        &mut self,
        chrome: OsdChrome,
        safe_insets: ViewInsets,
        ui_scale: f32,
        width: u32,
        height: u32,
    ) -> bool {
        let raster = (safe_insets, ui_scale.to_bits());
        let changed = self.last_chrome != Some(chrome)
            || self.last_raster != Some(raster)
            || self.last_size != Some((width, height));
        self.last_size = Some((width, height));
        self.last_chrome = Some(chrome);
        self.last_raster = Some(raster);
        changed
    }

    /// Decide the layer action for this frame: generation inequality means re-render.
    /// Status rides outside that flag: pause flips and toggles re-render, fps digits at 2Hz.
    pub fn sync(&mut self, state: &dyn OsdState, input: OsdSyncInput) -> OsdSync {
        let OsdSyncInput {
            tick,
            chrome,
            language,
            now,
            width,
            height,
            ui_scale,
            safe_insets,
        } = input;
        // Recorded only after the size decision, so a zero-size bump is never swallowed.
        let changed = state.generation() != self.last_seen;
        if width == 0 || height == 0 {
            self.last_size = None;
            if self.active {
                self.active = false;
                return OsdSync::Clear;
            }
            return OsdSync::Skip;
        }
        self.last_seen = state.generation();
        // Non-finite scales never reach the fingerprint: NaN would defeat `to_bits` equality.
        debug_assert!(ui_scale.is_finite(), "non-finite OSD scale");
        let ui_scale = if ui_scale.is_finite() { ui_scale } else { 1.0 };
        // Content decision first: generation plus status only.
        let status_changed = self.sync_status(&tick, &chrome, language, now);
        // Geometry only invalidates the cached plan; it never formats or boxes.
        let geometry_changed = self.sync_geometry(chrome, safe_insets, ui_scale, width, height);
        if !changed && !status_changed && !geometry_changed {
            return OsdSync::Skip;
        }
        let empty = state.visible_row().is_none() && self.last_status.is_none();
        if empty {
            if self.active {
                self.active = false;
                return OsdSync::Clear;
            }
            return OsdSync::Skip;
        }
        self.active = true;
        // Borrowed fingerprint: status strings share their allocation via `Arc`.
        let (label_value, position, status_position) = match self.last_status.as_ref() {
            Some(fingerprint) => (
                Some((
                    Arc::clone(&fingerprint.label),
                    Arc::clone(&fingerprint.value),
                )),
                fingerprint.position,
                fingerprint.status_position,
            ),
            None => (None, chrome.position, chrome.status_position),
        };
        OsdSync::Render(OsdPlan {
            rows: state.rows().to_vec(),
            status: label_value,
            position,
            status_position,
            width,
            height,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        OsdChrome, OsdPresenter, OsdRow, OsdState, OsdSync, OsdSyncInput, STATUS_FPS_MIN_INTERVAL,
        StatusGate, ViewInsets,
    };
    use crate::session::osd_presenter::TickMetrics;
    use nerust_settings_core::model::language::AppLanguage;
    use nerust_settings_core::model::local::NotificationSettings;
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

        fn rows(&self) -> &[OsdRow] {
            self.row.as_slice()
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
                text: text.into(),
                rgb: (255, 255, 255),
                tag: "[I]".into(),
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
        safe_insets: ViewInsets,
    ) -> OsdSyncInput {
        OsdSyncInput {
            tick: TickMetrics {
                fps: 60.0,
                ..TickMetrics::default()
            },
            chrome,
            language: AppLanguage::English,
            now,
            width,
            height,
            ui_scale,
            safe_insets,
        }
    }

    #[test]
    fn status_gate_throttles_digits_labels_and_fresh() {
        let mut gate = StatusGate::new();
        let now = Instant::now();
        // First sample passes.
        assert_eq!(
            gate.gate_or_cached(now, "", || "60.1 fps".to_string()),
            ("", "60.1 fps")
        );
        // Jitter within the interval holds the old digits.
        assert_eq!(
            gate.gate_or_cached(now, "", || "60.2 fps".to_string()),
            ("", "60.1 fps")
        );
        // After the interval the fresh digits pass.
        assert_eq!(
            gate.gate_or_cached(now + STATUS_FPS_MIN_INTERVAL, "", || {
                "60.2 fps".to_string()
            }),
            ("", "60.2 fps")
        );
        // A pause flip passes immediately even inside the interval.
        assert_eq!(
            gate.gate_or_cached(now + STATUS_FPS_MIN_INTERVAL, "Paused", || {
                "60.3 fps".to_string()
            }),
            ("Paused", "60.3 fps")
        );
        // Steady inputs never recompute the value: fresh runs only on change.
        let mut fresh_gate = StatusGate::new();
        let mut fresh_calls = 0;
        let (label, value) = fresh_gate.gate_or_cached(now, "Paused", || {
            fresh_calls += 1;
            "60.0 fps".to_string()
        });
        assert_eq!((label, value), ("Paused", "60.0 fps"));
        assert_eq!(fresh_calls, 1);
        let (_, steady) = fresh_gate.gate_or_cached(now, "Paused", || {
            fresh_calls += 1;
            "60.1 fps".to_string()
        });
        assert_eq!(steady, "60.0 fps");
        assert_eq!(fresh_calls, 1);
    }

    fn chrome(
        transient: (
            nerust_settings_core::model::local::NotificationAreaMode,
            nerust_settings_core::model::local::NotificationPosition,
        ),
        status: (
            nerust_settings_core::model::local::NotificationAreaMode,
            nerust_settings_core::model::local::NotificationPosition,
        ),
        status_on: bool,
    ) -> OsdChrome {
        OsdChrome {
            status_enabled: status_on,
            position: transient.1,
            status_position: status.1,
            area_mode: transient.0,
            status_area_mode: status.0,
        }
    }

    #[test]
    fn reserved_rows_assign_strips_by_edge_and_side() {
        use nerust_settings_core::model::local::{
            NotificationAreaMode as Mode, NotificationPosition as Pos,
        };
        // ((transient mode, transient corner), (status mode, status corner), status on) -> (top, bottom).
        for (transient, status, status_on, expected) in [
            (
                (Mode::Strip, Pos::BottomLeft),
                (Mode::Strip, Pos::BottomRight),
                true,
                (0, 1),
            ),
            (
                (Mode::Strip, Pos::BottomLeft),
                (Mode::Strip, Pos::BottomLeft),
                true,
                (0, 2),
            ),
            (
                (Mode::Overlay, Pos::BottomLeft),
                (Mode::Overlay, Pos::TopLeft),
                true,
                (0, 0),
            ),
            // Independent modes: overlay status reserves nothing.
            (
                (Mode::Strip, Pos::BottomLeft),
                (Mode::Overlay, Pos::TopLeft),
                true,
                (0, 1),
            ),
            // ... and an overlay transient leaves a strip status alone.
            (
                (Mode::Overlay, Pos::BottomLeft),
                (Mode::Strip, Pos::TopLeft),
                true,
                (1, 0),
            ),
            (
                (Mode::Overlay, Pos::TopLeft),
                (Mode::Overlay, Pos::BottomLeft),
                true,
                (0, 0),
            ),
            (
                (Mode::Strip, Pos::TopLeft),
                (Mode::Strip, Pos::BottomLeft),
                true,
                (1, 1),
            ),
            (
                (Mode::Strip, Pos::TopRight),
                (Mode::Strip, Pos::BottomRight),
                true,
                (1, 1),
            ),
            // Same corner stacks two rows on one side.
            (
                (Mode::Strip, Pos::TopLeft),
                (Mode::Strip, Pos::TopLeft),
                true,
                (2, 0),
            ),
            // Status off: transient row only.
            (
                (Mode::Strip, Pos::BottomLeft),
                (Mode::Strip, Pos::TopLeft),
                false,
                (0, 1),
            ),
            // Same band: the transient area mode governs the edge, so the
            // status picker cannot dangle an overlay inside a strip.
            (
                (Mode::Strip, Pos::BottomLeft),
                (Mode::Overlay, Pos::BottomLeft),
                true,
                (0, 2),
            ),
            (
                (Mode::Overlay, Pos::BottomLeft),
                (Mode::Strip, Pos::BottomLeft),
                true,
                (0, 0),
            ),
            (
                (Mode::Strip, Pos::BottomLeft),
                (Mode::Overlay, Pos::BottomRight),
                true,
                (0, 1),
            ),
            (
                (Mode::Overlay, Pos::BottomLeft),
                (Mode::Strip, Pos::BottomRight),
                true,
                (0, 0),
            ),
        ] {
            assert_eq!(
                chrome(transient, status, status_on).reserved_rows(),
                expected
            );
        }
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
            plan.status.as_ref().map(|(l, v)| (l.as_ref(), v.as_ref())),
            Some(("", "60.0 fps"))
        );
        assert_eq!(plan.width, 960);
        // Same inputs: fps jitter inside the gate holds the digits.
        assert!(matches!(
            presenter.sync(
                &state,
                OsdSyncInput {
                    tick: TickMetrics {
                        fps: 60.2,
                        ..TickMetrics::default()
                    },
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
                    tick: TickMetrics {
                        fps: 60.2,
                        paused: true,
                        ..TickMetrics::default()
                    },
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
        let plain = ViewInsets::default();
        let inset = ViewInsets {
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
    fn rerenders_on_position_change_with_status_off() {
        use nerust_settings_core::model::local::NotificationPosition as Pos;
        let state = row_state("Paused");
        let mut presenter = OsdPresenter::new();
        let settings = presenter_settings();
        assert!(!settings.status_enabled);
        let now = Instant::now();
        let input = |pos| {
            let mut chrome = chrome_of(&settings);
            chrome.position = pos;
            sync_input(chrome, now, 960, 720, 1.0, ViewInsets::default())
        };
        // Rows showing, status off: first frame renders.
        assert!(matches!(
            presenter.sync(&state, input(Pos::TopLeft)),
            OsdSync::Render(_)
        ));
        assert!(matches!(
            presenter.sync(&state, input(Pos::TopLeft)),
            OsdSync::Skip
        ));
        // Corner flip moves the layer: must re-render with identical
        // rows and no generation bump.
        assert!(matches!(
            presenter.sync(&state, input(Pos::BottomRight)),
            OsdSync::Render(_)
        ));
        assert!(matches!(
            presenter.sync(&state, input(Pos::BottomRight)),
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
                ViewInsets::default(),
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
    fn zero_size_then_same_size_republishes_unchanged_layer() {
        let state = row_state("Paused");
        let mut presenter = OsdPresenter::new();
        let settings = presenter_settings();
        let now = Instant::now();
        let input = |width| {
            sync_input(
                chrome_of(&settings),
                now,
                width,
                720,
                1.0,
                Default::default(),
            )
        };
        assert!(matches!(
            presenter.sync(&state, input(960)),
            OsdSync::Render(_)
        ));
        assert!(matches!(presenter.sync(&state, input(0)), OsdSync::Clear));
        assert!(matches!(presenter.sync(&state, input(0)), OsdSync::Skip));
        assert!(matches!(
            presenter.sync(&state, input(960)),
            OsdSync::Render(_)
        ));
        assert!(matches!(presenter.sync(&state, input(960)), OsdSync::Skip));
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
                    tick: TickMetrics {
                        fps: 60.0,
                        ..TickMetrics::default()
                    },
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
                    tick: TickMetrics {
                        fps: 61.0,
                        ..TickMetrics::default()
                    },
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
                tick: TickMetrics {
                    fps: 61.0,
                    ..TickMetrics::default()
                },
                now: off,
                ..sync_input(chrome_of(&settings), now, 960, 720, 1.0, Default::default())
            },
        ) {
            OsdSync::Render(plan) => plan,
            other => panic!("expected render, got {other:?}"),
        };
        assert_eq!(
            plan.status.as_ref().map(|(l, v)| (l.as_ref(), v.as_ref())),
            Some(("", "61.0 fps"))
        );
    }
}

#[cfg(test)]
mod presenter_cov_tests {
    use crate::session::osd_presenter::*;
    use nerust_settings_core::model::language::AppLanguage;

    #[test]
    fn tick_metrics_gate_pause_and_resume() {
        let running = TickMetrics {
            loaded: true,
            paused: false,
            ..TickMetrics::default()
        };
        assert!(running.can_pause());
        assert!(!running.can_resume());
        assert_eq!(running.frame_channel(), FrameChannel::Composited);
        let idle = TickMetrics::default();
        assert!(!idle.can_pause());
        assert!(!idle.can_resume());
        assert_eq!(idle.frame_channel(), FrameChannel::Unpresented);
    }

    #[test]
    fn status_key_and_text_cover_paused_and_running() {
        assert_eq!(status_key(true), "paused");
        assert_eq!(status_key(false), "");
        assert_eq!(status_text("paused", AppLanguage::English), "Paused");
        assert_eq!(status_text("paused", AppLanguage::Japanese), "一時停止中");
        assert_eq!(status_text("", AppLanguage::English), "");
        assert_eq!(status_text("other", AppLanguage::Japanese), "");
    }

    #[test]
    fn router_routes_transient_modal_and_drop() {
        use crate::session::notification::Message;
        let settings = NotificationSettings::default();
        let policy = TransientPolicy::from(&settings);
        let composited = ModalRouter {
            policy: &policy,
            channel: FrameChannel::Composited,
        };
        assert!(matches!(
            composited.route(&Message::paused()),
            ModalRoute::Transient
        ));
        let unpresented = ModalRouter {
            policy: &policy,
            channel: FrameChannel::Unpresented,
        };
        assert!(matches!(
            unpresented.route(&Message::paused()),
            ModalRoute::Drop
        ));
        assert!(matches!(
            unpresented.route(&Message::persistence_unavailable()),
            ModalRoute::Modal | ModalRoute::Transient
        ));
        let off = TransientPolicy {
            info_enabled: false,
            ..policy
        };
        let router = ModalRouter {
            policy: &off,
            channel: FrameChannel::Composited,
        };
        assert!(matches!(router.route(&Message::paused()), ModalRoute::Drop));
    }
}

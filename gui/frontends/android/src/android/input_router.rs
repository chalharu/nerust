//! InputRouter: touch overlay plus physical controller routing.
//! State lives in trackers; every JNI push routes through [`ChromePort`].

use nerust_gui_shell::session::SessionHandle;
use nerust_input_traits::AbstractKey;
use winit::{event::Touch, platform::android::activity::AndroidApp};

use super::{
    chrome_port, key_tracker::PhysicalKeyTracker, touch_layout::ProfileTouchOverlay,
    touch_tracker::TouchTracker,
};

/// Last pushed chrome state: rebuilds skip the JNI push when unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct PushedOverlay {
    visibility: String,
    opacity: u8,
    haptics: bool,
    zones: String,
    joystick: String,
}

/// Transient input state behind the overlay. All behavior that used to
/// live on `AndroidFrontend` now routes through here.
pub(super) struct InputRouter {
    overlay: Option<ProfileTouchOverlay>,
    tracker: TouchTracker,
    keys: PhysicalKeyTracker,
    overlay_revision: u64,
    pushed: Option<PushedOverlay>,
}

impl InputRouter {
    pub(super) fn new() -> Self {
        Self {
            overlay: None,
            tracker: TouchTracker::new(),
            keys: PhysicalKeyTracker::new(),
            overlay_revision: 0,
            pushed: None,
        }
    }

    /// Surface loss must not strand pressed buttons: release everything tracked.
    /// A stranded bit breaks exact-equality key checks (e.g. evolution cancel).
    pub(super) fn reset_touch_tracking(&mut self, session: &mut SessionHandle, app: &AndroidApp) {
        self.tracker.reset(session, app);
    }

    /// Release every physically held button before forgetting it.
    /// The overlay model is cached by revision and shared by every key.
    pub(super) fn release_physical_tracking(&mut self, session: &mut SessionHandle) {
        // Cached by revision: one clone per release, shared by every key.
        let model = session
            .touch_overlay_model_cached(self.overlay_revision)
            .clone();
        self.keys.release_all(session, &model);
    }

    /// Forget transient presses on focus loss without touching the
    /// emulator (the session already cleared its own input; Kotlin releases converge to no-ops).
    pub(super) fn forget_physical_pressed(&mut self) {
        self.keys.forget();
    }

    pub(super) fn apply_physical_controller_input(
        &mut self,
        session: &mut SessionHandle,
        device_id: i32,
        key: AbstractKey,
        pressed: bool,
    ) {
        // Cached by revision: one model build per layout, shared by every key.
        let model = session
            .touch_overlay_model_cached(self.overlay_revision)
            .clone();
        self.keys.apply(session, &model, device_id, key, pressed);
    }

    /// Rebuild the hit-zone overlay for the current window size and push it to Kotlin.
    /// Pushes the resolved zones plus the joystick home; Kotlin draws only these.
    pub(super) fn rebuild_overlay(
        &mut self,
        session: &SessionHandle,
        app: &AndroidApp,
        size: Option<(u32, u32)>,
    ) {
        let Some((width, height)) = size else {
            self.overlay = None;
            self.pushed = None;
            return;
        };
        let overlay_settings = &session.settings_snapshot().local.touch_overlay;
        self.overlay_revision = self.overlay_revision.wrapping_add(1);
        let model = session.touch_overlay_model(self.overlay_revision);
        self.overlay = if overlay_settings.visibility
            == nerust_settings_core::model::local::TouchOverlayVisibility::Hidden
        {
            None
        } else {
            Some(ProfileTouchOverlay::new(
                width as f32,
                height as f32,
                model.controls,
                overlay_settings.scale_percent,
                overlay_settings.vertical_offset_percent,
            ))
        };
        let zones = self
            .overlay
            .as_ref()
            .map(|overlay| {
                ProfileTouchOverlay::encode_zones(
                    &overlay.relative_zones(width as f32, height as f32),
                )
            })
            .unwrap_or_default();
        let joystick = self
            .overlay
            .as_ref()
            .map(|overlay| {
                ProfileTouchOverlay::encode_joystick(
                    &overlay.relative_joystick(width as f32, height as f32),
                )
            })
            .unwrap_or_default();
        // Coalesce: an unchanged overlay never re-crosses JNI.
        let pushed = PushedOverlay {
            visibility: format!("{:?}", overlay_settings.visibility),
            opacity: overlay_settings.opacity_percent,
            haptics: overlay_settings.haptics,
            zones,
            joystick,
        };
        if self.pushed.as_ref() == Some(&pushed) {
            return;
        }
        chrome_port::configure_overlay(
            app,
            overlay_settings,
            pushed.zones.clone(),
            pushed.joystick.clone(),
        );
        self.pushed = Some(pushed);
    }

    pub(super) fn handle_touch(
        &mut self,
        session: &mut SessionHandle,
        app: &AndroidApp,
        touch: Touch,
    ) {
        let haptics = session.settings_snapshot().local.touch_overlay.haptics;
        self.tracker
            .handle(session, app, self.overlay.as_ref(), haptics, touch);
    }
}

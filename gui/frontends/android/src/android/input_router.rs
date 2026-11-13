//! InputRouter: touch overlay plus physical controller routing.
//! State lives in trackers; every JNI push routes through [`ChromePort`].

use nerust_gui_shell::session::SessionHandle;
use nerust_input_traits::{PadGuid, SourceButtonId, SourceId, StandardButton};
use winit::{event::Touch, platform::android::activity::AndroidApp};

use super::{chrome_port, touch_layout::ProfileTouchOverlay, touch_tracker::TouchTracker};

/// Last pushed chrome state: rebuilds skip the JNI push when unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct PushedOverlay {
    visibility: std::sync::Arc<str>,
    opacity: u8,
    haptics: bool,
    zones: std::sync::Arc<str>,
    joystick: std::sync::Arc<str>,
}

/// Transient input state behind the overlay. All behavior that used to
/// live on `AndroidFrontend` now routes through here.
pub(super) struct InputRouter {
    overlay: Option<ProfileTouchOverlay>,
    tracker: TouchTracker,
    overlay_revision: u64,
    pushed: Option<PushedOverlay>,
    /// Live touch-response gate from Kotlin: drawing and responding share
    /// one rule, but only Kotlin knows the live half (gamepad presence,
    /// post-press quiet window). Closed on transitions; releases in flight.
    touch_respond: bool,
}

impl InputRouter {
    pub(super) fn new() -> Self {
        Self {
            overlay: None,
            tracker: TouchTracker::new(),
            overlay_revision: 0,
            pushed: None,
            touch_respond: true,
        }
    }

    /// Surface loss must not strand pressed buttons: release everything tracked.
    /// A stranded bit breaks exact-equality key checks (e.g. evolution cancel).
    pub(super) fn reset_touch_tracking(&mut self, session: &mut SessionHandle, app: &AndroidApp) {
        self.tracker.reset(session, app);
    }

    /// Pad source for one Android device id. The id is transient (the OS
    /// reuses it across reconnects), so disconnect releases converge first
    /// (Kotlin `releaseController`); this is routing, never persistence.
    fn pad_source(device_id: i32) -> SourceId {
        let mut bytes = [0u8; 16];
        bytes[..4].copy_from_slice(&device_id.to_le_bytes());
        bytes[4..8].copy_from_slice(b"ANDR");
        SourceId::Pad(PadGuid(bytes))
    }

    pub(super) fn apply_physical_controller_input(
        &mut self,
        session: &mut SessionHandle,
        device_id: i32,
        button: StandardButton,
        pressed: bool,
    ) {
        // Single session entry: repeat suppression and the OR merge live
        // in the session, so no adapter owns a private pressed set.
        session.handle_source_button(
            Self::pad_source(device_id),
            SourceButtonId::Pad(button),
            pressed,
        );
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
        // Unloaded frames push no zones: a stale D-Pad on a ROM-less
        // screen is confusing, and the memo below would otherwise
        // keep the previous ROM's zones alive across Unload.
        let loaded = session.tick_metrics().loaded;
        // System bars and cutouts live in the same physical pixels as the
        // window size; the layout confines the landscape band to the safe
        // box between them. Missing pushes (desktop tests, early startup)
        // mean zero insets, which reproduces the previous geometry exactly.
        let insets = super::bridge::cached_system_insets()
            .map(|insets| {
                use super::touch_layout::EdgeInsets;
                EdgeInsets {
                    top: insets[0].max(0) as f32,
                    right: insets[1].max(0) as f32,
                    bottom: insets[2].max(0) as f32,
                    left: insets[3].max(0) as f32,
                }
            })
            .unwrap_or_default();
        self.overlay = if !loaded
            || overlay_settings.visibility
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
                insets,
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
        // `Arc` shares the payload between the memo and the JNI closure.
        let pushed = PushedOverlay {
            visibility: std::sync::Arc::from(format!("{:?}", overlay_settings.visibility)),
            opacity: overlay_settings.opacity_percent,
            haptics: overlay_settings.haptics,
            zones: std::sync::Arc::from(zones),
            joystick: std::sync::Arc::from(joystick),
        };
        if self.pushed.as_ref() == Some(&pushed) {
            return;
        }
        chrome_port::configure_overlay(
            app,
            overlay_settings,
            std::sync::Arc::clone(&pushed.zones),
            std::sync::Arc::clone(&pushed.joystick),
        );
        self.pushed = Some(pushed);
    }

    /// Live response gate: invisible buttons never respond. Closing
    /// releases in-flight presses first so no bit strands held.
    pub(super) fn set_touch_respond(
        &mut self,
        session: &mut SessionHandle,
        app: &AndroidApp,
        respond: bool,
    ) {
        if self.touch_respond == respond {
            return;
        }
        self.touch_respond = respond;
        if !respond {
            self.tracker.reset(session, app);
        }
    }

    pub(super) fn handle_touch(
        &mut self,
        session: &mut SessionHandle,
        app: &AndroidApp,
        touch: Touch,
    ) {
        if !self.touch_respond {
            return;
        }
        let haptics = session.settings_snapshot().local.touch_overlay.haptics;
        self.tracker
            .handle(session, app, self.overlay.as_ref(), haptics, touch);
    }
}

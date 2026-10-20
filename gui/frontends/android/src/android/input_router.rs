//! InputRouter: touch overlay + physical controller routing.
//! Owns all transient input state and the overlay JNI push.

use std::collections::{HashMap, HashSet};

use jni::{jni_sig, jni_str};
use nerust_core_traits::touch::{
    TouchControlRole, TouchOverlayAction, TouchOverlayModel, TouchPoint,
};
use nerust_gui_shell::session::SessionHandle;
use nerust_input_traits::{AbstractKey, AttachmentId, DigitalControlId};
use winit::{event::Touch, platform::android::activity::AndroidApp};

use super::{
    bridge,
    touch_layout::{DpadTargets, FloatingDpadState, NO_DPAD_TARGETS, ProfileTouchOverlay},
};

/// Transient input state behind the overlay. All behavior that used to
/// live on `AndroidFrontend` now routes through here.
pub(super) struct InputRouter {
    overlay: Option<ProfileTouchOverlay>,
    active_touches: HashMap<u64, (AttachmentId, DigitalControlId)>,
    floating_dpad: Option<FloatingDpadState>,
    physical_pressed: HashSet<(i32, AbstractKey)>,
    overlay_revision: u64,
    /// Last pushed joystick visual, quantized to half pixels.
    /// Touch-move floods one JNI push per event; repeats are no-ops.
    last_dpad_visual: Option<(bool, i32, i32, i32, i32)>,
}

impl InputRouter {
    pub(super) fn new() -> Self {
        Self {
            overlay: None,
            active_touches: HashMap::new(),
            floating_dpad: None,
            physical_pressed: HashSet::new(),
            overlay_revision: 0,
            last_dpad_visual: None,
        }
    }

    /// Surface loss must not strand pressed buttons: release everything tracked.
    /// A stranded bit breaks exact-equality key checks (e.g. evolution cancel).
    pub(super) fn reset_touch_tracking(&mut self, session: &mut SessionHandle, app: &AndroidApp) {
        for (attachment, control) in self.active_touches.values() {
            session.apply_input_event(nerust_input_traits::DigitalInputEvent::released(
                *attachment,
                *control,
            ));
        }
        self.active_touches.clear();
        if let Some(state) = self.floating_dpad.take() {
            for (attachment, control) in state.targets.iter().flatten() {
                session.apply_input_event(nerust_input_traits::DigitalInputEvent::released(
                    *attachment,
                    *control,
                ));
            }
        }
        self.last_dpad_visual = None;
        update_floating_dpad_visual(
            app,
            false,
            TouchPoint { x: 0.0, y: 0.0 },
            TouchPoint { x: 0.0, y: 0.0 },
        );
    }

    /// Release every physically held button before forgetting it.
    /// The overlay model is built once and shared by every key.
    pub(super) fn release_physical_tracking(&mut self, session: &mut SessionHandle) {
        let model = session.touch_overlay_model(self.overlay_revision);
        for (_, key) in std::mem::take(&mut self.physical_pressed) {
            if let Some((attachment, control)) = Self::control_for_physical_key(&model, key) {
                session.apply_input_event(nerust_input_traits::DigitalInputEvent::released(
                    attachment, control,
                ));
            }
        }
    }

    /// Forget transient presses on focus loss without touching the
    /// emulator (the session already cleared its own input).
    pub(super) fn forget_physical_pressed(&mut self) {
        self.physical_pressed.clear();
    }

    /// Role lookup against a prebuilt overlay model.
    /// Callers build the model once per pass/event and share it across keys.
    fn control_for_physical_key(
        model: &TouchOverlayModel,
        key: AbstractKey,
    ) -> Option<(AttachmentId, DigitalControlId)> {
        let role = match key {
            AbstractKey::Button1 => TouchControlRole::FaceButton1,
            AbstractKey::Button2 => TouchControlRole::FaceButton2,
            AbstractKey::Button5 => TouchControlRole::LeftShoulder,
            AbstractKey::Button6 => TouchControlRole::RightShoulder,
            AbstractKey::Start => TouchControlRole::Start,
            AbstractKey::Select => TouchControlRole::Select,
            AbstractKey::DpadUp => TouchControlRole::DpadUp,
            AbstractKey::DpadDown => TouchControlRole::DpadDown,
            AbstractKey::DpadLeft => TouchControlRole::DpadLeft,
            AbstractKey::DpadRight => TouchControlRole::DpadRight,
            _ => return None,
        };
        model
            .controls
            .iter()
            .find(|control| control.role == role)
            .map(|control| (control.attachment_id, control.control_id))
    }

    pub(super) fn apply_physical_controller_input(
        &mut self,
        session: &mut SessionHandle,
        device_id: i32,
        key: AbstractKey,
        pressed: bool,
    ) {
        let changed = if pressed {
            self.physical_pressed.insert((device_id, key))
        } else {
            self.physical_pressed.remove(&(device_id, key))
        };
        if !changed {
            return;
        }
        let effective_pressed = self
            .physical_pressed
            .iter()
            .any(|(_, pressed_key)| *pressed_key == key);
        let model = session.touch_overlay_model(self.overlay_revision);
        let Some((attachment, control)) = Self::control_for_physical_key(&model, key) else {
            return;
        };
        let event = if effective_pressed {
            nerust_input_traits::DigitalInputEvent::pressed(attachment, control)
        } else {
            nerust_input_traits::DigitalInputEvent::released(attachment, control)
        };
        session.apply_input_event(event);
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
        configure_controls_overlay(app, overlay_settings, &zones, &joystick);
    }

    /// Single touch action dispatch: the old `Vec<TouchOverlayAction>`
    /// per touch event allocated on every move; callers pass one action.
    fn apply_touch_action(&mut self, session: &mut SessionHandle, action: TouchOverlayAction) {
        let TouchOverlayAction::Input(event) = action;
        session.apply_input_event(event);
    }

    fn sync_touch_target(
        &mut self,
        session: &mut SessionHandle,
        touch_id: u64,
        next_target: Option<(AttachmentId, DigitalControlId)>,
    ) {
        let previous = self.active_touches.get(&touch_id).copied();
        if previous == next_target {
            return;
        }
        if let Some(previous) = previous {
            self.apply_touch_action(
                session,
                TouchOverlayAction::Input(nerust_input_traits::DigitalInputEvent::released(
                    previous.0, previous.1,
                )),
            );
            self.active_touches.remove(&touch_id);
        }
        if let Some(next) = next_target {
            self.apply_touch_action(
                session,
                TouchOverlayAction::Input(nerust_input_traits::DigitalInputEvent::pressed(
                    next.0, next.1,
                )),
            );
            self.active_touches.insert(touch_id, next);
        }
    }

    fn sync_floating_dpad_targets(
        &mut self,
        session: &mut SessionHandle,
        next_targets: &DpadTargets,
    ) {
        let previous_targets = self
            .floating_dpad
            .as_ref()
            .map(|state| state.targets)
            .unwrap_or(NO_DPAD_TARGETS);
        for previous in previous_targets.iter().flatten() {
            if !next_targets.iter().flatten().any(|next| next == previous) {
                session.apply_input_event(nerust_input_traits::DigitalInputEvent::released(
                    previous.0, previous.1,
                ));
            }
        }
        for next in next_targets.iter().flatten() {
            if !previous_targets.iter().flatten().any(|prev| prev == next) {
                session.apply_input_event(nerust_input_traits::DigitalInputEvent::pressed(
                    next.0, next.1,
                ));
            }
        }
        if let Some(state) = self.floating_dpad.as_mut() {
            state.targets = *next_targets;
        }
    }

    fn begin_floating_dpad(
        &mut self,
        app: &AndroidApp,
        haptics: bool,
        touch_id: u64,
        point: TouchPoint,
    ) -> bool {
        let Some(overlay) = self.overlay.as_ref() else {
            return false;
        };
        if self.floating_dpad.is_some() || !overlay.accepts_floating_dpad(point) {
            return false;
        }
        self.floating_dpad = Some(FloatingDpadState {
            touch_id,
            center: point,
            targets: NO_DPAD_TARGETS,
        });
        log::info!(
            "floating D-Pad started: touch_id={touch_id} x={:.1} y={:.1}",
            point.x,
            point.y
        );
        self.push_dpad_visual(app, true, point, point);
        if haptics {
            perform_control_haptic(app);
        }
        true
    }

    fn update_floating_dpad(
        &mut self,
        session: &mut SessionHandle,
        app: &AndroidApp,
        point: TouchPoint,
    ) {
        let Some(center) = self.floating_dpad.as_ref().map(|state| state.center) else {
            return;
        };
        let Some(overlay) = self.overlay.as_ref() else {
            return;
        };
        let targets = overlay.floating_dpad_targets(center, point);
        let knob = overlay.floating_knob(center, point);
        self.sync_floating_dpad_targets(session, &targets);
        self.push_dpad_visual(app, true, center, knob);
    }

    /// Quantized visual push: touch-move floods events, but the JNI
    /// crossing only fires when the half-pixel key changes.
    fn push_dpad_visual(
        &mut self,
        app: &AndroidApp,
        active: bool,
        center: TouchPoint,
        knob: TouchPoint,
    ) {
        fn quantize(value: f32) -> i32 {
            (value * 2.0).round() as i32
        }
        let key = (
            active,
            quantize(center.x),
            quantize(center.y),
            quantize(knob.x),
            quantize(knob.y),
        );
        if self.last_dpad_visual == Some(key) {
            return;
        }
        self.last_dpad_visual = Some(key);
        update_floating_dpad_visual(app, active, center, knob);
    }

    fn end_floating_dpad(&mut self, session: &mut SessionHandle, app: &AndroidApp) {
        self.sync_floating_dpad_targets(session, &NO_DPAD_TARGETS);
        self.floating_dpad = None;
        log::info!("floating D-Pad ended");
        self.push_dpad_visual(
            app,
            false,
            TouchPoint { x: 0.0, y: 0.0 },
            TouchPoint { x: 0.0, y: 0.0 },
        );
    }

    pub(super) fn handle_touch(
        &mut self,
        session: &mut SessionHandle,
        app: &AndroidApp,
        touch: Touch,
    ) {
        let point = TouchPoint {
            x: touch.location.x as f32,
            y: touch.location.y as f32,
        };
        if self
            .floating_dpad
            .as_ref()
            .is_some_and(|state| state.touch_id == touch.id)
        {
            match touch.phase {
                winit::event::TouchPhase::Started | winit::event::TouchPhase::Moved => {
                    self.update_floating_dpad(session, app, point)
                }
                winit::event::TouchPhase::Ended | winit::event::TouchPhase::Cancelled => {
                    self.end_floating_dpad(session, app)
                }
            }
            return;
        }
        let haptics = session.settings_snapshot().local.touch_overlay.haptics;
        if touch.phase == winit::event::TouchPhase::Started
            && self.begin_floating_dpad(app, haptics, touch.id, point)
        {
            return;
        }
        let next_target = self
            .overlay
            .as_ref()
            .and_then(|overlay| overlay.hit_test(point));
        match touch.phase {
            winit::event::TouchPhase::Started | winit::event::TouchPhase::Moved => {
                if touch.phase == winit::event::TouchPhase::Started
                    && next_target.is_some()
                    && haptics
                {
                    perform_control_haptic(app);
                }
                self.sync_touch_target(session, touch.id, next_target);
            }
            winit::event::TouchPhase::Ended | winit::event::TouchPhase::Cancelled => {
                self.sync_touch_target(session, touch.id, None);
            }
        }
    }
}

pub(super) fn configure_controls_overlay(
    app: &AndroidApp,
    settings: &nerust_settings_core::model::local::TouchOverlaySettings,
    zones: &str,
    joystick: &str,
) {
    let visibility = match settings.visibility {
        nerust_settings_core::model::local::TouchOverlayVisibility::Always => "always",
        nerust_settings_core::model::local::TouchOverlayVisibility::Auto => "auto",
        nerust_settings_core::model::local::TouchOverlayVisibility::Hidden => "hidden",
    }
    .to_string();
    let opacity = i32::from(settings.opacity_percent);
    let haptics = settings.haptics;
    let zones = zones.to_string();
    let joystick = joystick.to_string();
    bridge::call_on_activity(
        app,
        "configure Android controls overlay",
        move |env, activity| {
            let visibility = env.new_string(&visibility)?;
            let zones = env.new_string(&zones)?;
            let joystick = env.new_string(&joystick)?;
            env.call_method(
                &activity,
                jni_str!("configureControlsOverlay"),
                jni_sig!("(Ljava/lang/String;IZZLjava/lang/String;Ljava/lang/String;)V"),
                &[
                    jni::objects::JValue::Object(visibility.as_ref()),
                    jni::objects::JValue::Int(opacity),
                    jni::objects::JValue::Bool(haptics),
                    jni::objects::JValue::Object(zones.as_ref()),
                    jni::objects::JValue::Object(joystick.as_ref()),
                ],
            )?;
            Ok(())
        },
    );
}

pub(super) fn perform_control_haptic(app: &AndroidApp) {
    bridge::call_on_activity(app, "play Android control haptic", |env, activity| {
        env.call_method(
            &activity,
            jni_str!("performControlHaptic"),
            jni_sig!("()V"),
            &[],
        )?;
        Ok(())
    });
}

pub(super) fn update_floating_dpad_visual(
    app: &AndroidApp,
    active: bool,
    center: TouchPoint,
    knob: TouchPoint,
) {
    bridge::call_on_activity(app, "update Android dpad visual", move |env, activity| {
        env.call_method(
            &activity,
            jni_str!("updateFloatingDpad"),
            jni_sig!("(ZFFFF)V"),
            &[
                jni::objects::JValue::Bool(active),
                jni::objects::JValue::Float(center.x),
                jni::objects::JValue::Float(center.y),
                jni::objects::JValue::Float(knob.x),
                jni::objects::JValue::Float(knob.y),
            ],
        )?;
        Ok(())
    });
}

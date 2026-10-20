//! TouchTracker: transient touch state and floating-D-Pad visuals.
//! Hit-testing reads the overlay built by the router; pushes go to the port.

use std::collections::HashMap;

use nerust_core_traits::touch::{TouchOverlayAction, TouchPoint};
use nerust_gui_shell::session::SessionHandle;
use nerust_input_traits::{AttachmentId, DigitalControlId};
use winit::{event::Touch, platform::android::activity::AndroidApp};

use super::{
    chrome_port,
    touch_layout::{DpadTargets, FloatingDpadState, NO_DPAD_TARGETS, ProfileTouchOverlay},
};

/// Active touches, floating-D-Pad tracking, and visual quantization.
/// A touch-move flood pushes JNI only when the quantized visual changes.
pub(super) struct TouchTracker {
    active_touches: HashMap<u64, (AttachmentId, DigitalControlId)>,
    floating_dpad: Option<FloatingDpadState>,
    /// Last pushed joystick visual, quantized to half pixels.
    /// Touch-move floods one JNI push per event; repeats are no-ops.
    last_dpad_visual: Option<(bool, i32, i32, i32, i32)>,
}

impl TouchTracker {
    pub(super) fn new() -> Self {
        Self {
            active_touches: HashMap::new(),
            floating_dpad: None,
            last_dpad_visual: None,
        }
    }

    /// Surface loss must not strand pressed buttons: release everything tracked.
    /// A stranded bit breaks exact-equality key checks (e.g. evolution cancel).
    pub(super) fn reset(&mut self, session: &mut SessionHandle, app: &AndroidApp) {
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
        chrome_port::dpad_visual(
            app,
            false,
            TouchPoint { x: 0.0, y: 0.0 },
            TouchPoint { x: 0.0, y: 0.0 },
        );
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
        overlay: &ProfileTouchOverlay,
    ) -> bool {
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
            chrome_port::haptic(app);
        }
        true
    }

    fn update_floating_dpad(
        &mut self,
        session: &mut SessionHandle,
        app: &AndroidApp,
        point: TouchPoint,
        overlay: &ProfileTouchOverlay,
    ) {
        let Some(center) = self.floating_dpad.as_ref().map(|state| state.center) else {
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
        chrome_port::dpad_visual(app, active, center, knob);
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

    pub(super) fn handle(
        &mut self,
        session: &mut SessionHandle,
        app: &AndroidApp,
        overlay: Option<&ProfileTouchOverlay>,
        haptics: bool,
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
                    if let Some(overlay) = overlay {
                        self.update_floating_dpad(session, app, point, overlay);
                    }
                }

                winit::event::TouchPhase::Ended | winit::event::TouchPhase::Cancelled => {
                    self.end_floating_dpad(session, app);
                }
            }
            return;
        }
        if touch.phase == winit::event::TouchPhase::Started
            && let Some(overlay) = overlay
            && self.begin_floating_dpad(app, haptics, touch.id, point, overlay)
        {
            return;
        }
        let next_target = overlay.and_then(|overlay| overlay.hit_test(point));
        match touch.phase {
            winit::event::TouchPhase::Started | winit::event::TouchPhase::Moved => {
                if touch.phase == winit::event::TouchPhase::Started
                    && next_target.is_some()
                    && haptics
                {
                    chrome_port::haptic(app);
                }
                self.sync_touch_target(session, touch.id, next_target);
            }
            winit::event::TouchPhase::Ended | winit::event::TouchPhase::Cancelled => {
                self.sync_touch_target(session, touch.id, None);
            }
        }
    }
}

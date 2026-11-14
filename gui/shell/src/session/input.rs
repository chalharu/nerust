use nerust_core_traits::touch::{TouchControl, TouchControlRole, TouchOverlayModel};
use nerust_input_traits::{
    AbstractKey, AttachmentId, ControlId, DigitalControlId, DigitalInputEvent, GamepadAxis,
    InputValue, Key, SourceButtonId, SourceId, StandardButton,
};
use nerust_settings_core::model::input::ShortcutAction;

use crate::{
    session::{KeyboardShortcut, SessionHandle, field_press::FieldSource},
    settings::bindings::events::shortcut::shortcut_action_for_key,
};

pub use nerust_settings_core::input::clear_multi_port_conflicts;

/// Reserved frontend device id for winit-delivered pad keys: winit hides
/// the Android device id behind an opaque handle, so those keys share one
/// transient namespace until a descriptor arrives to collapse it.
const WINIT_PAD_DEVICE_ID: i32 = i32::MIN;

fn touch_role_for_abstract_key(key: AbstractKey) -> Option<TouchControlRole> {
    match key {
        AbstractKey::DpadUp => Some(TouchControlRole::DpadUp),
        AbstractKey::DpadDown => Some(TouchControlRole::DpadDown),
        AbstractKey::DpadLeft => Some(TouchControlRole::DpadLeft),
        AbstractKey::DpadRight => Some(TouchControlRole::DpadRight),
        AbstractKey::Button1 => Some(TouchControlRole::FaceButton1),
        AbstractKey::Button2 => Some(TouchControlRole::FaceButton2),
        AbstractKey::Button5 => Some(TouchControlRole::LeftShoulder),
        AbstractKey::Button6 => Some(TouchControlRole::RightShoulder),
        AbstractKey::Start => Some(TouchControlRole::Start),
        AbstractKey::Select => Some(TouchControlRole::Select),
        _ => None,
    }
}

impl SessionHandle {
    /// Last served overlay, keyed by caller revision and the session topology epoch.
    pub fn touch_overlay_model_cached(&mut self, revision: u64) -> &TouchOverlayModel {
        if self.input.touch_cache.rev != Some(revision)
            || self.input.touch_cache.topology_epoch != self.input.topology_epoch
        {
            let model = self.touch_overlay_model(revision);
            self.input.touch_cache = super::TouchCache {
                rev: Some(revision),
                topology_epoch: self.input.topology_epoch,
                model,
            };
        }
        &self.input.touch_cache.model
    }

    pub fn touch_overlay_model(&self, revision: u64) -> TouchOverlayModel {
        let cache = &self.input.touch_cache;
        let served_revision = if cache.rev.is_none() {
            revision
        } else if cache.rev == Some(revision) && cache.topology_epoch == self.input.topology_epoch {
            cache.model.revision
        } else {
            cache.model.revision.wrapping_add(1)
        };
        build_touch_overlay(served_revision, &self.input.current_assignments)
    }
}

/// Pure overlay builder: assignments in, model out.
/// Both session entry points delegate here; the mapping lives in one place.
fn build_touch_overlay(
    revision: u64,
    assignments: &nerust_input_traits::InputAssignments,
) -> TouchOverlayModel {
    let mut controls = Vec::with_capacity(assignments.slots().len());
    let mut seen = std::collections::HashSet::new();
    for (attachment_id, profile) in assignments.slots() {
        let Some(profile) = profile else {
            continue;
        };
        let port_set = profile
            .port_sets()
            .iter()
            .find(|set| set.ports.contains(attachment_id));
        let Some(port_set) = port_set else {
            continue;
        };
        for (&port, group) in port_set.ports.iter().zip(profile.port_groups()) {
            for info in *group {
                let Some(control_id) = info.id.as_digital() else {
                    continue;
                };
                let Some(role) = info.abstract_key.and_then(touch_role_for_abstract_key) else {
                    continue;
                };
                if !seen.insert((port, control_id)) {
                    continue;
                }
                controls.push(TouchControl {
                    attachment_id: port,
                    control_id,
                    role,
                    label: std::sync::Arc::from(info.label),
                });
            }
        }
    }
    TouchOverlayModel { revision, controls }
}

impl SessionHandle {
    /// Shared press/release tail: the OR/neutral machine resolves updates,
    /// then they apply to the live input in one place.
    fn commit_press(
        &mut self,
        field: usize,
        source: FieldSource,
        dpad: Option<(AttachmentId, DigitalControlId, AbstractKey)>,
        pressed: bool,
    ) {
        let updates = if pressed {
            self.input.press_state.press(field, source, dpad)
        } else {
            self.input.press_state.release(field, source, dpad)
        };
        if let Some(gui_input) = self.emu.input_mut() {
            for &(field, assert) in updates.as_slice() {
                let _ = gui_input.set(field, InputValue::Digital(assert));
            }
        }
    }

    /// Called by touch overlay (Android) with a pre-resolved DigitalInputEvent.
    /// Routes through the OR/neutral machine so multi-device holds merge and
    /// opposite dpad directions neutralize instead of sticking.
    pub fn apply_input_event(&mut self, event: DigitalInputEvent) {
        let field = self.emu.field_map().and_then(|fields| {
            fields
                .get(&(event.attachment, ControlId::from(event.control)))
                .copied()
        });
        let Some(field) = field else {
            return;
        };
        let dpad = self.input.dpad_for(event.attachment, event.control);
        self.commit_press(
            field,
            FieldSource::Control(event.attachment, event.control),
            dpad,
            event.is_pressed(),
        );
    }

    pub fn handle_keyboard_key(&mut self, key: Key, pressed: bool) -> Option<KeyboardShortcut> {
        self.handle_source_button(SourceId::KEYBOARD, SourceButtonId::Key(key), pressed)
    }

    /// Map one shortcut action to the frontend command. Shared by keyboard
    /// and pad lookups so the two columns never diverge in meaning.
    fn shortcut_kind(action: ShortcutAction) -> KeyboardShortcut {
        if matches!(action, ShortcutAction::ToggleFullscreen) {
            KeyboardShortcut::ToggleFullscreen
        } else if matches!(action, ShortcutAction::OpenDebugger) {
            KeyboardShortcut::OpenDebugger
        } else {
            KeyboardShortcut::Session(action)
        }
    }

    /// Pad shortcut lookup: the pad column of the unified binding, or
    /// `None` when the button drives gameplay instead. First binding wins;
    /// shared buttons surface as pad conflicts and block submit until
    /// disambiguated.
    fn shortcut_for_pad(&self, button: StandardButton) -> Option<KeyboardShortcut> {
        self.settings
            .snapshot
            .shared
            .input
            .shortcuts
            .keyboard
            .iter()
            .find(|binding| binding.pad == Some(button))
            .map(|binding| Self::shortcut_kind(binding.action))
    }

    /// Assigned player slot for one event source, registering first-seen
    /// pads. `None` parks the source (Off, keyboards on the axis path,
    /// or pads past the persisted table).
    fn pad_slot_for(&mut self, source: SourceId) -> Option<usize> {
        let ports = &self.settings.snapshot.shared.input.gamepad.pad_ports;
        self.input.pad_slot(source, ports)
    }

    /// Stable source for one frontend device id: the connect-time
    /// descriptor when the connect event arrived, else a transient id key.
    /// The router builds every pad source here, never from raw ids, so a
    /// reconnected pad (reused id) keeps its order index and slot. No
    /// allocation: both namespaces are `Copy` GUIDs.
    pub fn pad_source_for(&mut self, device_id: i32) -> SourceId {
        let guid = self
            .input
            .device_ids
            .get(&device_id)
            .copied()
            .unwrap_or_else(|| super::InputDept::guid_for_transient(device_id));
        self.input.pad_index_for_guid(guid);
        SourceId::Pad(guid)
    }

    /// Source for winit-delivered pad keys: winit hides the Android device
    /// id behind an opaque handle, so this pins those keys to the
    /// first-seen pad (the connected one in single-pad play). Multi-pad
    /// winit keys converge here too; per-pad distinction for that path
    /// needs NDK polling and stays future work. Dual delivery with the
    /// Kotlin bridge is safe: the session pressed set suppresses repeats.
    pub fn primary_pad_source(&mut self) -> SourceId {
        if let Some(guid) = self.input.pad_order.first().copied() {
            return SourceId::Pad(guid);
        }
        self.pad_source_for(WINIT_PAD_DEVICE_ID)
    }

    /// Connect-time device registration: descriptor to order index.
    /// Later input from any reused device id resolves to the same pad.
    /// A transient placeholder for this id (input before connect) collapses
    /// into the descriptor entry; holds keyed by it converge on the next
    /// focus-loss clear.
    pub fn note_pad_connected(&mut self, device_id: i32, descriptor: &str) {
        if descriptor.is_empty() {
            return;
        }
        let guid = super::InputDept::guid_for_descriptor(descriptor);
        self.input.device_ids.insert(device_id, guid);
        // Collapse a transient placeholder for this id, but never move an
        // existing descriptor: reconnects reuse the original index.
        let transient = super::InputDept::guid_for_transient(device_id);
        self.input.pad_order.retain(|known| *known != transient);
        if !self.input.pad_order.contains(&guid) {
            self.input.pad_order.push(guid);
        }
    }

    /// Disconnect-time cleanup: drop the id mapping (Kotlin already
    /// synthesized the releases). The order entry stays so a reconnect
    /// reuses the index instead of drifting to a new pad.
    pub fn note_pad_disconnected(&mut self, device_id: i32) {
        self.input.device_ids.remove(&device_id);
    }

    /// Single entry for every physical source. Repeat suppression is one
    /// `(source, button)` set: keyboards and pads share the rule, so no
    /// adapter owns a private pressed set. Unbound pad buttons drive
    /// gameplay; bound ones resolve through the shortcut pad column.
    pub fn handle_source_button(
        &mut self,
        source: SourceId,
        button: SourceButtonId,
        pressed: bool,
    ) -> Option<KeyboardShortcut> {
        let first_press = if pressed {
            self.input.pressed.insert((source, button))
        } else {
            self.input.pressed.remove(&(source, button));
            false
        };

        // Repeats change nothing (the source already holds the field);
        // releases always re-sync so a stuck field cannot survive.
        let changed = (pressed && first_press) || !pressed;
        match button {
            SourceButtonId::Key(key) => {
                if changed && let Some(&field) = self.input.key_field_map.get(&key) {
                    // Copy ids out: no allocation on the hot path.
                    let dpad =
                        self.input.key_source.get(&key).copied().and_then(
                            |(attachment, control)| self.input.dpad_for(attachment, control),
                        );
                    self.commit_press(field, FieldSource::Key(key), dpad, pressed);
                }

                if first_press {
                    return shortcut_action_for_key(&self.settings.snapshot.shared, key)
                        .map(Self::shortcut_kind);
                }
                None
            }
            SourceButtonId::Pad(std_button) => {
                if changed {
                    self.commit_pad_press(source, std_button, pressed);
                }
                if pressed && first_press {
                    return self.shortcut_for_pad(std_button);
                }
                None
            }
        }
    }

    /// Pad press tail: position button to the pad's assigned console
    /// control. Off-port pads never reach here (the slot gate parks them).
    fn commit_pad_press(&mut self, source: SourceId, button: StandardButton, pressed: bool) {
        let slot = self.pad_slot_for(source);
        self.commit_holder_press(FieldSource::Pad(source, button), button, slot, pressed);
    }

    /// Full-press level turning a trigger into a digital press when the
    /// pad opts into shoulder doubling. Post-deadzone units.
    const TRIGGER_FULL_PRESS: f32 = 0.9;

    /// Substitute direction for one post-deadzone axis value. Sticks stand
    /// in for dpad on digital-only consoles; triggers only when the pad
    /// opts into shoulder doubling, else they stay analog-only.
    fn substitute_button(
        axis: GamepadAxis,
        value: f32,
        triggers_as_shoulders: bool,
    ) -> Option<StandardButton> {
        use StandardButton::{DpadDown, DpadLeft, DpadRight, DpadUp};
        match axis {
            GamepadAxis::LeftStickX | GamepadAxis::RightStickX => {
                if value < 0.0 {
                    Some(DpadLeft)
                } else if value > 0.0 {
                    Some(DpadRight)
                } else {
                    None
                }
            }
            GamepadAxis::LeftStickY | GamepadAxis::RightStickY => {
                if value < 0.0 {
                    Some(DpadUp)
                } else if value > 0.0 {
                    Some(DpadDown)
                } else {
                    None
                }
            }
            GamepadAxis::LeftTrigger => (triggers_as_shoulders
                && value >= Self::TRIGGER_FULL_PRESS)
                .then_some(StandardButton::LeftShoulder),
            GamepadAxis::RightTrigger => (triggers_as_shoulders
                && value >= Self::TRIGGER_FULL_PRESS)
                .then_some(StandardButton::RightShoulder),
        }
    }

    /// Analog axis entry: deadzone, store, analog-field set, and the
    /// digital substitute (stick dpad on digital-only consoles, trigger
    /// shoulders when the pad opts in). The substitute needs an assigned
    /// slot, so Off-port pads park their floats silently.
    pub fn handle_source_axis(&mut self, source: SourceId, axis: GamepadAxis, value: f32) {
        let slot = self.pad_slot_for(source);
        // Per-pad deadzone: sticks vary per unit, so the pad's own row
        // tunes it. Unmapped sources fall back to the default.
        let percent = self
            .input
            .pad_index(source)
            .and_then(|index| {
                self.settings
                    .snapshot
                    .shared
                    .input
                    .gamepad
                    .pad_ports
                    .get(index)
                    .map(|port| port.deadzone_percent)
            })
            .unwrap_or(nerust_settings_core::model::input::DEFAULT_DEADZONE_PERCENT)
            .clamp(0, 90);
        let value =
            nerust_gamepad_db::apply_deadzone(value.clamp(-1.0, 1.0), f32::from(percent) / 100.0)
                .clamp(-1.0, 1.0);
        self.input.analogs.insert((source, axis), value);
        if let Some(abstract_key) = axis.abstract_key()
            && let Some(slot) = slot
            && let Some((attachment, control)) = self
                .input
                .control_for_analog_abstract(abstract_key, Some(slot))
            && let Some(field) = self.emu.field_map().and_then(|fields| {
                fields
                    .get(&(attachment, ControlId::analog(control)))
                    .copied()
            })
            && let Some(gui_input) = self.emu.input_mut()
        {
            let _ = gui_input.set(field, InputValue::Analog(value));
        }
        let alias = self
            .input
            .pad_index(source)
            .and_then(|index| {
                self.settings
                    .snapshot
                    .shared
                    .input
                    .gamepad
                    .pad_ports
                    .get(index)
            })
            .is_some_and(|port| port.triggers_as_shoulders);
        self.sync_axis_substitute(source, axis, value, slot, alias);
    }

    /// Edge-sync one axis substitute: press the newly deflected button,
    /// release the stale one. Axis holders stay distinct from button
    /// holders, so hat and stick (or shoulder and trigger) merge instead
    /// of aliasing.
    fn sync_axis_substitute(
        &mut self,
        source: SourceId,
        axis: GamepadAxis,
        value: f32,
        slot: Option<usize>,
        triggers_as_shoulders: bool,
    ) {
        let want = slot.and(Self::substitute_button(axis, value, triggers_as_shoulders));
        // One axis holds at most one substitute (presses are edge-synced),
        // so a single find converges without heap work on the hot path.
        if let Some(stale) = self
            .input
            .axis_dpad
            .iter()
            .find(|(held_source, held_axis, _)| *held_source == source && *held_axis == axis)
            .map(|(_, _, button)| *button)
            .filter(|button| Some(*button) != want)
        {
            self.input.axis_dpad.remove(&(source, axis, stale));
            self.commit_holder_press(FieldSource::Axis(source, axis), stale, slot, false);
        }
        if let Some(button) = want
            && self.input.axis_dpad.insert((source, axis, button))
        {
            self.commit_holder_press(FieldSource::Axis(source, axis), button, slot, true);
        }
    }

    /// One digital commit through the pad's assigned slot: face buttons
    /// and axis substitutes share the resolve and differ only in the
    /// OR-machine holder, so hat and stick merge instead of aliasing.
    /// Off-port (and over-table) pads park here: without an assigned
    /// slot there is no control to drive, overrides or not.
    /// Per-pad overrides win inside the slot: an explicit `(attachment,
    /// control)` for this pad's button bypasses the standard layout.
    fn commit_holder_press(
        &mut self,
        holder: FieldSource,
        button: StandardButton,
        slot: Option<usize>,
        pressed: bool,
    ) {
        let Some(slot) = slot else {
            return;
        };
        let resolved = self
            .input
            .pad_index(match holder {
                FieldSource::Pad(source, _) => source,
                FieldSource::Axis(source, _) => source,
                _ => return,
            })
            .and_then(|index| {
                self.input
                    .pad_binding_map
                    .get(index)
                    .and_then(|map| map.get(&button).copied())
            });
        if let Some((attachment, control)) = resolved {
            let field = self
                .emu
                .field_map()
                .and_then(|fields| fields.get(&(attachment, ControlId::from(control))).copied());
            let Some(field) = field else {
                return;
            };
            let dpad = self.input.dpad_for(attachment, control);
            self.commit_press(field, holder, dpad, pressed);
            return;
        }
        let Some(abstract_key) = button.abstract_key() else {
            return;
        };
        let Some((attachment, control)) = self.input.control_for_abstract(abstract_key, Some(slot))
        else {
            return;
        };
        let field = self
            .emu
            .field_map()
            .and_then(|fields| fields.get(&(attachment, ControlId::from(control))).copied());
        let Some(field) = field else {
            return;
        };
        let dpad = self.input.dpad_for(attachment, control);
        self.commit_press(field, holder, dpad, pressed);
    }

    /// Event-ful release of every held pad button (surface loss): releases
    /// travel through the OR machine so fields held by other sources stay
    /// asserted. Stick-derived holds release too, and stored analogs drop
    /// (only pads ever store them; keyboards have no axis path).
    pub fn release_pad_presses(&mut self) {
        let held: Vec<(SourceId, StandardButton)> = self
            .input
            .pressed
            .iter()
            .filter_map(|(source, button)| match button {
                SourceButtonId::Pad(std_button) => Some((*source, *std_button)),
                SourceButtonId::Key(_) => None,
            })
            .collect();
        for (source, button) in held {
            self.input
                .pressed
                .remove(&(source, SourceButtonId::Pad(button)));
            self.commit_pad_press(source, button, false);
        }
        let derived: Vec<(SourceId, GamepadAxis, StandardButton)> =
            self.input.axis_dpad.iter().copied().collect();
        for (source, axis, button) in derived {
            self.input.axis_dpad.remove(&(source, axis, button));
            let slot = self.pad_slot_for(source);
            self.commit_holder_press(FieldSource::Axis(source, axis), button, slot, false);
        }
        self.input.analogs.clear();
    }

    /// Re-assert every held pad button through the live assignments.
    /// Pair with the keyboard re-assert in `rebind_input`: the live
    /// fields were just cleared, so both sources survive a settings
    /// apply without a re-press. Pads re-resolve (ports may have moved);
    /// Off-parked holds simply assert nothing.
    pub(super) fn reassert_pad_presses(&mut self) {
        let held: Vec<(SourceId, StandardButton)> = self
            .input
            .pressed
            .iter()
            .filter_map(|(source, button)| match button {
                SourceButtonId::Pad(std_button) => Some((*source, *std_button)),
                SourceButtonId::Key(_) => None,
            })
            .collect();
        for (source, button) in held {
            self.commit_pad_press(source, button, true);
        }
    }

    pub fn clear_input(&mut self) {
        self.input.pressed.clear();
        self.input.analogs.clear();
        self.input.axis_dpad.clear();
        self.input.press_state.clear();
        if let Some(gui_input) = self.emu.input_mut() {
            gui_input.clear();
        }
    }

    pub fn rebuild_key_field_map(&mut self) {
        let system = self
            .active_system_id_box()
            .and_then(|system_id| self.settings.snapshot.shared.input.systems.get(&system_id));
        let bindings = system.and_then(|s| s.implicit_keyboard_profile());
        let pad_bindings = system.map(|s| &s.pad_bindings);
        // The department owns the maps; the session only resolves which
        // profile is active.
        self.input.rebuild_key_field_map(
            bindings.map(|p| p.bindings.as_slice()),
            self.emu.field_map(),
        );
        self.input.rebuild_pad_binding_map(pad_bindings);
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, rc::Rc};

    use nerust_gui_viewmodel::settings::build_topology;
    use nerust_input_traits::{
        AbstractKey, AttachmentId, ControlId, ControlInfo, ControlKind, ControllerProfile,
        DeviceKindId, DigitalControlId, InputAssignments, PortSet, ProfileId, SlotInfo,
        UnifiedFieldMap,
    };
    use nerust_settings_core::model::input::KeyboardBinding;
    use nerust_settings_core::model::input::PersistedControlId;

    use super::*;
    use crate::session::input_map::rebuild_input_map;

    /// Single-port mock: "test.standard" profile with one port.
    #[derive(Debug)]
    struct MockSinglePort;
    impl ControllerProfile for MockSinglePort {
        fn profile_id(&self) -> ProfileId {
            ProfileId::new("test.standard")
        }
        fn label(&self) -> &'static str {
            "Test Standard"
        }
        fn port_sets(&self) -> &[PortSet] {
            static P: &[AttachmentId] = &[AttachmentId::new("test.slot")];
            static S: &[PortSet] = &[PortSet { ports: P }];
            S
        }
        fn port_groups(&self) -> &[&[ControlInfo]] {
            &[]
        }
    }

    /// Multi-port mock: "test.multi" profile with P1/P2 ports in one set.
    #[derive(Debug)]
    struct MockMultiPort;
    impl ControllerProfile for MockMultiPort {
        fn profile_id(&self) -> ProfileId {
            ProfileId::new("test.multi")
        }
        fn label(&self) -> &'static str {
            "Test Multi"
        }
        fn port_sets(&self) -> &[PortSet] {
            static P: &[AttachmentId] =
                &[AttachmentId::new("test.p1"), AttachmentId::new("test.p2")];
            static S: &[PortSet] = &[PortSet { ports: P }];
            S
        }
        fn port_groups(&self) -> &[&[ControlInfo]] {
            &[]
        }
    }

    #[derive(Debug)]
    struct MockTopologyProfile;

    impl ControllerProfile for MockTopologyProfile {
        fn profile_id(&self) -> ProfileId {
            ProfileId::new("test.topology")
        }

        fn label(&self) -> &'static str {
            "Topology Pad"
        }

        fn port_sets(&self) -> &[PortSet] {
            static PORTS: &[AttachmentId] = &[AttachmentId::new("test.topology.slot")];
            static SETS: &[PortSet] = &[PortSet { ports: PORTS }];
            SETS
        }

        fn port_groups(&self) -> &[&[ControlInfo]] {
            static CONTROLS: &[ControlInfo] = &[ControlInfo {
                id: ControlId::digital(DigitalControlId::new("test.topology.a")),
                label: "A",
                kind: ControlKind::Digital,
                abstract_key: Some(AbstractKey::Button1),
            }];
            static GROUPS: &[&[ControlInfo]] = &[CONTROLS];
            GROUPS
        }

        fn device_kind_for_group(&self, _group_index: usize) -> DeviceKindId {
            DeviceKindId::new("test.topology.pad")
        }
    }

    #[test]
    fn clear_multi_port_does_nothing_for_single_port() {
        let profile = MockSinglePort;
        let slot = AttachmentId::new("test.slot");
        let mut assignments: Vec<(AttachmentId, Option<Rc<dyn ControllerProfile>>)> =
            vec![(slot, Some(Rc::new(MockSinglePort)))];
        clear_multi_port_conflicts(slot, &profile, &mut assignments);
        assert!(assignments[0].1.is_some());
    }

    #[test]
    fn clear_multi_port_clears_other_ports() {
        let profile = MockMultiPort;
        let p1 = AttachmentId::new("test.p1");
        let p2 = AttachmentId::new("test.p2");
        let mut assignments: Vec<(AttachmentId, Option<Rc<dyn ControllerProfile>>)> = vec![
            (p1, Some(Rc::new(MockMultiPort))),
            (p2, Some(Rc::new(MockMultiPort))),
        ];
        clear_multi_port_conflicts(p1, &profile, &mut assignments);
        assert!(assignments[0].1.is_some(), "P1 should stay assigned");
        assert!(assignments[1].1.is_none(), "P2 should be cleared");
    }

    #[test]
    fn clear_multi_port_does_not_clear_unrelated() {
        let profile = MockMultiPort;
        let p1 = AttachmentId::new("test.p1");
        let p2 = AttachmentId::new("test.p2");
        let other = AttachmentId::new("test.other");
        let mut assignments: Vec<(AttachmentId, Option<Rc<dyn ControllerProfile>>)> = vec![
            (other, Some(Rc::new(MockSinglePort))),
            (p1, Some(Rc::new(MockMultiPort))),
            (p2, Some(Rc::new(MockMultiPort))),
        ];
        clear_multi_port_conflicts(p1, &profile, &mut assignments);
        assert!(assignments[0].1.is_some(), "Unrelated port unchanged");
        assert!(assignments[1].1.is_some(), "P1 stays");
        assert!(assignments[2].1.is_none(), "P2 cleared");
    }

    #[test]
    fn build_topology_returns_empty_model_without_controllers() {
        let topology = build_topology(&[], &[]);

        assert!(topology.ports().is_empty());
        assert!(topology.devices().is_empty());
    }

    #[test]
    fn build_topology_describes_assigned_controller() {
        let slot = AttachmentId::new("test.topology.slot");
        let profile: Rc<dyn ControllerProfile> = Rc::new(MockTopologyProfile);
        let topology = build_topology(
            &[(slot, Some(profile))],
            &[SlotInfo {
                id: slot,
                label: "Topology Port",
            }],
        );

        assert_eq!(topology.ports().len(), 1);
        assert_eq!(topology.ports()[0].label(), "Topology Port");
        assert_eq!(topology.devices().len(), 1);
        assert_eq!(topology.devices()[0].kind().as_str(), "test.topology.pad");
        assert_eq!(topology.devices()[0].controls().len(), 1);
    }

    #[test]
    fn rebuild_input_map_maps_matching_keyboard_binding() {
        let attachment = AttachmentId::new("test.topology.slot");
        let control = ControlId::digital(DigitalControlId::new("test.topology.a"));
        let mut field_map = UnifiedFieldMap::new();
        let _ = field_map.insert((attachment, control), 7);
        let bindings = [KeyboardBinding::new(
            attachment.as_str(),
            PersistedControlId::digital(control.as_str()),
            Key::KeyA,
        )];
        let mut key_map = HashMap::from([(Key::KeyB, 2)]);

        rebuild_input_map(&field_map, &bindings, &mut key_map).expect("no duplicates");

        assert_eq!(key_map, HashMap::from([(Key::KeyA, 7)]));
    }

    #[test]
    fn physical_key_conflicts_are_deterministic_and_analog_ids_do_not_alias() {
        let slot = AttachmentId::new("test.slot");
        let a = DigitalControlId::new("test.a");
        let b = DigitalControlId::new("test.b");
        let bindings = [
            KeyboardBinding::new(
                slot.as_str(),
                PersistedControlId::analog(a.as_str()),
                Key::KeyB,
            ),
            KeyboardBinding::new(
                slot.as_str(),
                PersistedControlId::digital(a.as_str()),
                Key::KeyA,
            ),
            KeyboardBinding::new(
                slot.as_str(),
                PersistedControlId::digital(b.as_str()),
                Key::KeyA,
            ),
        ];
        for reverse in [false, true] {
            let mut entries = vec![
                ((slot, ControlId::digital(a)), 1),
                ((slot, ControlId::digital(b)), 2),
            ];
            if reverse {
                entries.reverse();
            }
            let fields = nerust_input_traits::collect_first_wins(entries);
            let mut result = HashMap::new();
            let error = rebuild_input_map(&fields, &bindings, &mut result).unwrap_err();
            assert_eq!(error.control, "test.b");
            assert_eq!(result, HashMap::from([(Key::KeyA, 1)]));
        }
    }

    #[test]
    fn source_buttons_share_one_suppression_set_and_pads_release_alone() {
        use nerust_input_traits::{SourceButtonId, StandardButton};
        let mut session = crate::session::test_util::test_session();
        let pad = session.pad_source_for(11);
        let south = SourceButtonId::Pad(StandardButton::South);
        session.handle_source_button(pad, south, true);
        session.handle_keyboard_key(Key::KeyA, true);
        assert!(session.input.pressed.contains(&(pad, south)));
        // Pad repeats change nothing; the release below still re-syncs.
        session.handle_source_button(pad, south, true);
        session.release_pad_presses();
        assert!(
            !session
                .input
                .pressed
                .iter()
                .any(|(_, button)| matches!(button, SourceButtonId::Pad(_)))
        );
        assert!(
            session
                .input
                .pressed
                .contains(&(SourceId::KEYBOARD, SourceButtonId::Key(Key::KeyA)))
        );
        session.clear_input();
    }

    #[test]
    fn pad_resolution_uses_live_assignments_first_port_wins() {
        use nerust_input_traits::StandardButton;
        let mut session = crate::session::test_util::test_session();
        session.input.current_assignments = InputAssignments::new(vec![(
            AttachmentId::new("test.topology.slot"),
            Some(Rc::new(MockTopologyProfile)),
        )]);
        let found = session
            .input
            .control_for_abstract(StandardButton::South.abstract_key().expect("mapped"), None);
        assert_eq!(
            found.expect("topology control").0,
            AttachmentId::new("test.topology.slot")
        );
        // A pinned slot outside the assignments resolves nothing.
        assert!(
            session
                .input
                .control_for_abstract(
                    StandardButton::South.abstract_key().expect("mapped"),
                    Some(3)
                )
                .is_none()
        );
        assert!(
            session
                .input
                .control_for_abstract(AbstractKey::Guide, None)
                .is_none()
        );
    }

    #[test]
    fn pad_guide_triggers_pause_shortcut() {
        use nerust_input_traits::{SourceButtonId, StandardButton};
        use nerust_settings_core::model::input::{ShortcutAction, ShortcutBinding};
        let mut session = crate::session::test_util::test_session();
        let pad = session.pad_source_for(11);
        // Unbound pad buttons drive gameplay (None), never a shortcut.
        assert!(
            session
                .handle_source_button(pad, SourceButtonId::Pad(StandardButton::South), true)
                .is_none()
        );
        let mut next = session.settings_snapshot().clone();
        next.shared.input.shortcuts.keyboard = vec![ShortcutBinding {
            action: ShortcutAction::TogglePause,
            key: None,
            pad: Some(StandardButton::Guide),
        }];
        session.apply_settings(next).unwrap();
        let shortcut =
            session.handle_source_button(pad, SourceButtonId::Pad(StandardButton::Guide), true);
        assert!(matches!(
            shortcut,
            Some(KeyboardShortcut::Session(ShortcutAction::TogglePause))
        ));
        session.clear_input();
    }

    #[test]
    fn pad_overrides_rebuild_from_active_system_settings() {
        use nerust_input_traits::StandardButton;
        use nerust_settings_core::model::input::{
            PadBinding, PersistedControlId, SystemInputSettings,
        };
        let mut session = crate::session::test_util::test_session();
        let system = session.active_system_id_box().expect("test system active");
        // No overrides by default: the maps stay empty.
        session.rebuild_key_field_map();
        assert!(
            session
                .input
                .pad_binding_map
                .iter()
                .all(|map| map.is_empty())
        );
        // One digital override on pad 0; the analog entry never lands.
        let mut system_settings = SystemInputSettings::default();
        system_settings.pad_bindings[0] = vec![
            PadBinding::new(
                "test.topology.slot",
                PersistedControlId::digital("test.topology.a"),
                StandardButton::East,
            ),
            PadBinding::new(
                "test.topology.slot",
                PersistedControlId::analog("test.topology.x"),
                StandardButton::West,
            ),
        ];
        let mut next = session.settings_snapshot().clone();
        next.shared.input.systems.insert(system, system_settings);
        session.apply_settings(next).unwrap();
        let map = &session.input.pad_binding_map[0];
        assert_eq!(map.len(), 1, "analog entries never reach the map");
        assert_eq!(
            map[&StandardButton::East],
            (
                AttachmentId::new("test.topology.slot"),
                DigitalControlId::new("test.topology.a")
            )
        );
        assert!(
            session.input.pad_binding_map[1].is_empty(),
            "pads stay independent"
        );
    }

    #[test]
    fn pad_override_duplicates_keep_first_like_keyboard() {
        use nerust_input_traits::StandardButton;
        use nerust_settings_core::model::input::{
            PadBinding, PersistedControlId, SystemInputSettings,
        };
        let mut session = crate::session::test_util::test_session();
        let system = session.active_system_id_box().expect("test system active");
        let mut system_settings = SystemInputSettings::default();
        system_settings.pad_bindings[0] = vec![
            PadBinding::new(
                "test.topology.slot",
                PersistedControlId::digital("test.topology.a"),
                StandardButton::East,
            ),
            PadBinding::new(
                "test.topology.slot",
                PersistedControlId::digital("test.topology.b"),
                StandardButton::East,
            ),
        ];
        let mut next = session.settings_snapshot().clone();
        next.shared.input.systems.insert(system, system_settings);
        session.apply_settings(next).unwrap();
        let map = &session.input.pad_binding_map[0];
        assert_eq!(map.len(), 1);
        assert_eq!(
            map[&StandardButton::East].1,
            DigitalControlId::new("test.topology.a"),
            "first entry wins, the duplicate only logs"
        );
    }

    #[test]
    fn axis_deadzone_reads_per_pad_rows() {
        use nerust_input_traits::GamepadAxis;
        let mut session = crate::session::test_util::test_session();
        let pad0 = session.pad_source_for(11);
        let pad1 = session.pad_source_for(12);
        let mut next = session.settings_snapshot().clone();
        next.shared.input.gamepad.pad_ports[0].deadzone_percent = 0;
        next.shared.input.gamepad.pad_ports[1].deadzone_percent = 90;
        session.apply_settings(next).unwrap();
        session.handle_source_axis(pad0, GamepadAxis::LeftStickX, 0.5);
        session.handle_source_axis(pad1, GamepadAxis::LeftStickX, 0.5);
        let stored0 = session.input.analogs[&(pad0, GamepadAxis::LeftStickX)];
        let stored1 = session.input.analogs[&(pad1, GamepadAxis::LeftStickX)];
        assert!((stored0 - 0.5).abs() < 1e-6, "zero deadzone stores raw");
        assert_eq!(stored1, 0.0, "wide deadzone swallows the deflection");
        session.clear_input();
    }

    #[test]
    fn axis_events_store_deadzoned_values_and_substitute_stick_dpad() {
        use nerust_input_traits::{GamepadAxis, StandardButton};
        let mut session = crate::session::test_util::test_session();
        session.input.current_assignments = InputAssignments::new(vec![(
            AttachmentId::new("test.topology.slot"),
            Some(Rc::new(MockTopologyProfile)),
        )]);
        let pad = session.pad_source_for(11);
        session.handle_source_axis(pad, GamepadAxis::LeftStickX, 0.5);
        let stored = session
            .input
            .analogs
            .get(&(pad, GamepadAxis::LeftStickX))
            .copied()
            .expect("axis value stored");
        assert!(
            (stored
                - nerust_gamepad_db::apply_deadzone(
                    0.5,
                    f32::from(nerust_settings_core::model::input::DEFAULT_DEADZONE_PERCENT) / 100.0
                ))
            .abs()
                < 1e-6
        );
        assert!(session.input.axis_dpad.contains(&(
            pad,
            GamepadAxis::LeftStickX,
            StandardButton::DpadRight
        )));
        // Recenter clears the substitute; the trigger stores yet never
        // substitutes (no console exposes it).
        session.handle_source_axis(pad, GamepadAxis::LeftStickX, 0.0);
        assert!(
            !session
                .input
                .axis_dpad
                .iter()
                .any(|(source, axis, _)| *source == pad && *axis == GamepadAxis::LeftStickX)
        );
        session.handle_source_axis(pad, GamepadAxis::LeftTrigger, 0.9);
        assert!(
            session
                .input
                .analogs
                .get(&(pad, GamepadAxis::LeftTrigger))
                .copied()
                .expect("trigger stored")
                > 0.0
        );
        assert!(session.input.axis_dpad.is_empty());
        session.clear_input();
        assert!(session.input.analogs.is_empty());
        assert!(session.input.axis_dpad.is_empty());
    }

    #[test]
    fn off_port_pads_park_their_substitute() {
        use nerust_input_traits::GamepadAxis;
        let mut session = crate::session::test_util::test_session();
        session.input.current_assignments = InputAssignments::new(vec![(
            AttachmentId::new("test.topology.slot"),
            Some(Rc::new(MockTopologyProfile)),
        )]);
        let mut next = session.settings_snapshot().clone();
        next.shared.input.gamepad.pad_ports = Default::default();
        session.apply_settings(next).unwrap();
        let pad = session.pad_source_for(11);
        session.handle_source_axis(pad, GamepadAxis::LeftStickX, 0.8);
        assert!(
            session
                .input
                .analogs
                .contains_key(&(pad, GamepadAxis::LeftStickX))
        );
        assert!(session.input.axis_dpad.is_empty());
        session.clear_input();
    }

    #[test]
    fn connect_collapses_transient_placeholders() {
        let mut session = crate::session::test_util::test_session();
        // Input before the connect event parks on a transient key.
        let transient = session.pad_source_for(7);
        // The connect collapses it: the same pad on a reused id resolves
        // to one descriptor source, with no placeholder left behind.
        session.note_pad_connected(7, "pad-desc-A");
        session.note_pad_connected(9, "pad-desc-A");
        assert_eq!(session.pad_source_for(9), session.pad_source_for(7));
        assert_ne!(transient, session.pad_source_for(7));
        assert!(
            !session
                .input
                .pad_order
                .contains(&crate::session::InputDept::guid_for_transient(7))
        );
        session.clear_input();
    }

    #[test]
    fn descriptor_identity_survives_id_reuse_and_reconnects() {
        let mut session = crate::session::test_util::test_session();
        session.note_pad_connected(5, "pad-desc-A");
        let first = session.pad_source_for(5);
        session.note_pad_disconnected(5);
        // Same descriptor on a reused id resolves to the same pad.
        session.note_pad_connected(9, "pad-desc-A");
        assert_eq!(session.pad_source_for(9), first);
        // A different descriptor is a different pad.
        session.note_pad_connected(9, "pad-desc-B");
        assert_ne!(session.pad_source_for(9), first);
        // Unknown ids park on transient keys, never on a real pad.
        let transient = session.pad_source_for(77);
        assert_ne!(transient, first);
        session.clear_input();
    }

    #[test]
    fn trigger_alias_doubles_shoulders_only_when_opted_in() {
        use nerust_input_traits::{GamepadAxis, StandardButton};
        let mut session = crate::session::test_util::test_session();
        session.input.current_assignments = InputAssignments::new(vec![(
            AttachmentId::new("test.topology.slot"),
            Some(Rc::new(MockTopologyProfile)),
        )]);
        let pad = session.pad_source_for(11);
        // Default prefs: analog-only, full press stores but asserts nothing.
        session.handle_source_axis(pad, GamepadAxis::LeftTrigger, 1.0);
        assert!(session.input.axis_dpad.is_empty());
        // Opt pad 0 into shoulder doubling: full press now holds the
        // shoulder through a distinct axis holder.
        let mut next = session.settings_snapshot().clone();
        next.shared.input.gamepad.pad_ports[0].triggers_as_shoulders = true;
        session.apply_settings(next).unwrap();
        session.handle_source_axis(pad, GamepadAxis::LeftTrigger, 1.0);
        assert!(session.input.axis_dpad.contains(&(
            pad,
            GamepadAxis::LeftTrigger,
            StandardButton::LeftShoulder
        )));
        session.handle_source_axis(pad, GamepadAxis::LeftTrigger, 0.0);
        assert!(session.input.axis_dpad.is_empty());
        session.clear_input();
    }

    #[test]
    fn settings_apply_reasserts_pad_holds_without_repress() {
        use nerust_input_traits::{SourceButtonId, StandardButton};
        let mut session = crate::session::test_util::test_session();
        session.input.current_assignments = InputAssignments::new(vec![(
            AttachmentId::new("test.topology.slot"),
            Some(Rc::new(MockTopologyProfile)),
        )]);
        let pad = session.pad_source_for(11);
        session.handle_source_button(pad, SourceButtonId::Pad(StandardButton::South), true);
        assert!(
            session
                .input
                .pressed
                .contains(&(pad, SourceButtonId::Pad(StandardButton::South)))
        );
        // Unrelated apply clears the live fields; the hold re-asserts
        // through the new assignments instead of going silent.
        let fullscreen = session
            .settings_snapshot()
            .local
            .video
            .window
            .fullscreen_default;
        session.set_fullscreen_default(!fullscreen).unwrap();
        assert!(
            session
                .input
                .pressed
                .contains(&(pad, SourceButtonId::Pad(StandardButton::South))),
            "pad holds survive a settings apply like keyboard holds do"
        );
        session.handle_source_button(pad, SourceButtonId::Pad(StandardButton::South), false);
        assert!(
            !session
                .input
                .pressed
                .iter()
                .any(|(_, button)| matches!(button, SourceButtonId::Pad(_)))
        );
        session.clear_input();
    }

    #[test]
    fn unrelated_fullscreen_apply_preserves_physical_key_state() {
        let mut session = crate::session::test_util::test_session();
        session.handle_keyboard_key(Key::KeyA, true);
        let fullscreen = session
            .settings_snapshot()
            .local
            .video
            .window
            .fullscreen_default;
        session.set_fullscreen_default(!fullscreen).unwrap();
        use nerust_input_traits::{SourceButtonId, SourceId};
        let probe = (SourceId::KEYBOARD, SourceButtonId::Key(Key::KeyA));
        assert!(session.input.pressed.contains(&probe));
        session.handle_keyboard_key(Key::KeyA, false);
        assert!(!session.input.pressed.contains(&probe));
    }

    #[test]
    fn rebuild_input_map_reports_first_duplicate_for_caller_to_log() {
        use crate::session::input_map::DuplicateBinding;
        let attachment = AttachmentId::new("test.topology.slot");
        let control = ControlId::digital(DigitalControlId::new("test.topology.a"));
        let mut field_map = UnifiedFieldMap::new();
        let _ = field_map.insert((attachment, control), 7);
        let bindings = [
            KeyboardBinding::new(
                attachment.as_str(),
                PersistedControlId::digital(control.as_str()),
                Key::KeyA,
            ),
            KeyboardBinding::new(
                attachment.as_str(),
                PersistedControlId::digital(control.as_str()),
                Key::KeyB,
            ),
        ];
        let mut key_map = HashMap::new();

        let duplicate = rebuild_input_map(&field_map, &bindings, &mut key_map)
            .expect_err("second binding duplicates the first");

        // First registration wins; the duplicate is reported, not logged here.
        assert_eq!(key_map, HashMap::from([(Key::KeyA, 7)]));
        assert_eq!(
            duplicate,
            DuplicateBinding {
                attachment: attachment.as_str().to_string(),
                control: control.as_str().to_string(),
            }
        );
    }

    #[test]
    fn touch_overlay_uses_assigned_profile_controls() {
        // Pure builder under test: assignments in, model out. The session
        // entry points delegate here, so no session mutation is needed.
        let slot = AttachmentId::new("test.topology.slot");
        let overlay = super::build_touch_overlay(
            42,
            &InputAssignments::new(vec![(slot, Some(Rc::new(MockTopologyProfile)))]),
        );

        assert_eq!(overlay.revision, 42);
        assert_eq!(overlay.controls.len(), 1);
        assert_eq!(overlay.controls[0].attachment_id, slot);
        assert_eq!(overlay.controls[0].control_id.as_str(), "test.topology.a");
        assert_eq!(overlay.controls[0].role, TouchControlRole::FaceButton1);
        assert_eq!(overlay.controls[0].label.as_ref(), "A");

        // Empty assignments serve the empty overlay without a session.
        let empty = super::build_touch_overlay(7, &InputAssignments::new(vec![]));
        assert_eq!(empty.revision, 7);
        assert!(empty.controls.is_empty());
    }

    #[test]
    fn settings_commit_invalidates_overlay_even_with_same_caller_revision() {
        let mut session = crate::session::test_util::test_session();
        session.input.current_assignments = InputAssignments::new(vec![(
            AttachmentId::new("test.topology.slot"),
            Some(Rc::new(MockTopologyProfile)),
        )]);
        let before = session.touch_overlay_model_cached(42).clone();
        assert_eq!(before.controls.len(), 1);
        let next = session.settings_snapshot().clone();
        session.apply_settings(next).unwrap();
        let expected = session.touch_overlay_model(42);
        assert_ne!(expected, before);
        assert_eq!(session.touch_overlay_model_cached(42), &expected);
        assert_ne!(
            expected.revision, before.revision,
            "frontend must observe the topology change"
        );
    }

    #[test]
    fn topology_epoch_republishes_even_when_controls_are_equal() {
        let mut session = crate::session::test_util::test_session();
        let before = session.touch_overlay_model_cached(42).clone();
        session.input.invalidate_topology();
        let after = session.touch_overlay_model_cached(42).clone();
        assert_eq!(before.controls, after.controls);
        assert_ne!(before.revision, after.revision);
        assert_eq!(session.touch_overlay_model_cached(42), &after);
    }

    #[test]
    fn multi_port_overlay_uses_group_within_selected_port_set() {
        #[derive(Debug)]
        struct TwoPorts;
        impl ControllerProfile for TwoPorts {
            fn profile_id(&self) -> ProfileId {
                ProfileId::new("two")
            }
            fn label(&self) -> &'static str {
                "two"
            }
            fn port_sets(&self) -> &[PortSet] {
                static PORTS: &[AttachmentId] = &[AttachmentId::new("p1"), AttachmentId::new("p2")];
                static SETS: &[PortSet] = &[PortSet { ports: PORTS }];
                SETS
            }
            fn port_groups(&self) -> &[&[ControlInfo]] {
                static A: &[ControlInfo] = &[ControlInfo {
                    id: ControlId::digital(DigitalControlId::new("a")),
                    label: "A",
                    kind: ControlKind::Digital,
                    abstract_key: Some(AbstractKey::Button1),
                }];
                static B: &[ControlInfo] = &[ControlInfo {
                    id: ControlId::digital(DigitalControlId::new("b")),
                    label: "B",
                    kind: ControlKind::Digital,
                    abstract_key: Some(AbstractKey::Button2),
                }];
                static GROUPS: &[&[ControlInfo]] = &[A, B];
                GROUPS
            }
        }
        let primary_only =
            InputAssignments::new(vec![(AttachmentId::new("p1"), Some(Rc::new(TwoPorts)))]);
        let assignments = InputAssignments::new(vec![
            (AttachmentId::new("p1"), Some(Rc::new(TwoPorts))),
            (AttachmentId::new("p2"), Some(Rc::new(TwoPorts))),
        ]);
        let model = build_touch_overlay(0, &assignments);
        assert_eq!(
            build_touch_overlay(0, &primary_only),
            model,
            "a multi-port profile occupies all ports even when assigned only at its primary slot"
        );
        assert_eq!(model.controls.len(), 2);
        assert_eq!(model.controls[0].control_id.as_str(), "a");
        assert_eq!(model.controls[1].control_id.as_str(), "b");
        assert_eq!(model.controls[1].role, TouchControlRole::FaceButton2);
    }

    #[test]
    fn touch_role_maps_shoulder_buttons() {
        assert_eq!(
            touch_role_for_abstract_key(AbstractKey::Button5),
            Some(TouchControlRole::LeftShoulder)
        );
        assert_eq!(
            touch_role_for_abstract_key(AbstractKey::Button6),
            Some(TouchControlRole::RightShoulder)
        );
        assert_eq!(
            touch_role_for_abstract_key(AbstractKey::Button1),
            Some(TouchControlRole::FaceButton1)
        );
        assert_eq!(touch_role_for_abstract_key(AbstractKey::Guide), None);
    }
}

#[cfg(test)]
mod input_cov_tests {
    use crate::session::input::*;
    use crate::session::test_util::test_session;

    #[test]
    fn touch_role_maps_every_direction() {
        for (key, role) in [
            (AbstractKey::DpadUp, TouchControlRole::DpadUp),
            (AbstractKey::DpadDown, TouchControlRole::DpadDown),
            (AbstractKey::DpadLeft, TouchControlRole::DpadLeft),
            (AbstractKey::DpadRight, TouchControlRole::DpadRight),
            (AbstractKey::Button2, TouchControlRole::FaceButton2),
            (AbstractKey::Start, TouchControlRole::Start),
            (AbstractKey::Select, TouchControlRole::Select),
        ] {
            assert_eq!(touch_role_for_abstract_key(key), Some(role));
        }
        assert_eq!(touch_role_for_abstract_key(AbstractKey::Guide), None);
    }

    #[test]
    fn keyboard_press_release_tracks_first_press() {
        let mut session = test_session();
        let first = session.handle_keyboard_key(Key::KeyA, true);
        let _ = first;
        let repeat = session.handle_keyboard_key(Key::KeyA, true);
        assert!(repeat.is_none());
        let released = session.handle_keyboard_key(Key::KeyA, false);
        assert!(released.is_none());
        session.clear_input();
    }

    #[test]
    fn apply_input_event_without_mapping_is_noop() {
        use nerust_input_traits::{AttachmentId, DigitalInputEvent};
        let mut session = test_session();
        session.apply_input_event(DigitalInputEvent::pressed(
            AttachmentId::new("nope.slot"),
            nerust_input_traits::DigitalControlId::new("nope.btn"),
        ));
        session.clear_input();
        session.rebuild_key_field_map();
    }

    #[test]
    fn overlay_cache_serves_stored_model() {
        let mut session = test_session();
        let model = session.touch_overlay_model_cached(11).clone();
        assert_eq!(model.revision, 11);
        assert_eq!(session.touch_overlay_model_cached(11).revision, 11);
        assert_eq!(session.touch_overlay_model_cached(12).revision, 12);
    }
}

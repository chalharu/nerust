use nerust_core_traits::touch::{TouchControl, TouchControlRole, TouchOverlayModel};
use nerust_input_traits::{
    AbstractKey, AttachmentId, ControlId, DigitalControlId, DigitalInputEvent, GamepadAxis,
    InputValue, Key, SourceButtonId, SourceId, StandardButton, apply_deadzone,
};
use nerust_settings_core::model::input::{
    DEADZONE_MAX_PERCENT, DEFAULT_DEADZONE_PERCENT, ShortcutAction,
};

use crate::session::{KeyboardShortcut, SessionHandle, field_press::FieldSource};

pub use nerust_settings_core::input::clear_multi_port_conflicts;

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
                let Some(role) = info.abstract_key.and_then(TouchControlRole::for_abstract) else {
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
        // Virtual ids never consume order slots: a ghost entry would
        // squat a pad index and shut out a real pad past the table.
        if device_id < 0 {
            return SourceId::Pad(super::PadTopology::guid_for_transient(device_id));
        }
        let guid = self
            .input
            .pads
            .device_ids
            .get(&device_id)
            .copied()
            .unwrap_or_else(|| super::PadTopology::guid_for_transient(device_id));
        self.input.pads.pad_index_for_guid(guid);
        SourceId::Pad(guid)
    }

    /// Source for winit-delivered pad keys: the first *assigned* pad, or
    /// `None` when no assigned pad exists and the caller must drop the
    /// keys. Winit hides the Android device id behind an opaque handle,
    /// so per-pad distinction for that path needs NDK polling and stays
    /// future work. Assigned-first (not first-seen) so a parked pad 0
    /// cannot swallow pad 1's keys. Dual delivery with the Kotlin bridge
    /// resolves through `shortcut_held`, so one physical press fires its
    /// shortcut once no matter which path arrives first. Never mints
    /// a descriptor-less transient: that would pin winit keys to a
    /// parked pad and let one player drive another. Skips transients by
    /// rank (see `pad_index`): a pre-connect key never steals pad 0.
    pub fn primary_pad_source(&self) -> Option<SourceId> {
        let ports = &self.settings.snapshot.shared.input.gamepad.pad_ports;
        let mut rank = 0;
        for guid in &self.input.pads.order {
            if super::PadTopology::is_transient(guid) {
                continue;
            }
            if self.input.slot_for_index(rank, ports).is_some() {
                return Some(SourceId::Pad(*guid));
            }
            rank += 1;
        }
        None
    }

    /// Connect-time device registration: descriptor to order index.
    /// Later input from any reused device id resolves to the same pad.
    /// A transient placeholder for this id (input before connect) collapses
    /// into the descriptor entry in place, keeping its order index (and
    /// slot); holders keyed by it migrate to the descriptor source so
    /// pre-connect holds keep driving instead of stranding on a dead key.
    /// The descriptor mirror tracks the same slots (transients skipped),
    /// so the settings vault sorts its display list into session order
    /// and a reconnect can never skew a pad onto another pad's settings.
    pub fn note_pad_connected(&mut self, device_id: i32, descriptor: &str) {
        if descriptor.is_empty() || device_id < 0 {
            return;
        }
        let guid = super::PadTopology::guid_for_descriptor(descriptor);
        self.input.pads.device_ids.insert(device_id, guid);
        let transient = super::PadTopology::guid_for_transient(device_id);
        if transient == guid {
            return;
        }
        let old = SourceId::Pad(transient);
        let new = SourceId::Pad(guid);
        if let Some(pos) = self
            .input
            .pads
            .order
            .iter()
            .position(|known| *known == transient)
        {
            if self.input.pads.order.contains(&guid) {
                // An existing descriptor never moves: drop the placeholder.
                // The mirror never stored transients, so nothing drops there.
                self.input.pads.order.remove(pos);
            } else {
                // Collapse in place, but mirror by setting rank: stray
                // transients ahead of `pos` never shift the vault list.
                let rank = self.input.pads.setting_rank(pos);
                self.input.pads.order[pos] = guid;
                self.input.pads.upsert_descriptor(Some(rank), descriptor);
            }
            self.migrate_pad_source(old, new);
        } else if !self.input.pads.order.contains(&guid) {
            self.input.pads.order.push(guid);
            self.input.pads.upsert_descriptor(None, descriptor);
        }
    }

    /// Session-order descriptors for the settings vault: `descriptors[i]`
    /// is pad `i`'s device descriptor. The vault sorts its display list by
    /// this, so rows, radios, and capture targets name session indices.
    pub fn pad_descriptors(&self) -> &[String] {
        &self.input.pads.descriptor_order
    }

    /// Rewrite holders keyed by a collapsed transient source to the
    /// descriptor source: pressed buttons, stored analogs, and
    /// stick-derived dpad. Without this, holds from before the connect
    /// event strand on a dead key until the next focus-loss clear.
    fn migrate_pad_source(&mut self, old: SourceId, new: SourceId) {
        if old == new {
            return;
        }
        Self::migrate_set(&mut self.input.pressed, old, new);
        Self::migrate_values(&mut self.input.analogs, old, new);
        // `axis_dpad` keys are triples; the generic covers pairs only.
        let derived: Vec<(GamepadAxis, StandardButton)> = self
            .input
            .axis_dpad
            .iter()
            .filter_map(|(source, axis, button)| (*source == old).then_some((*axis, *button)))
            .collect();
        for (axis, button) in derived {
            if self.input.axis_dpad.remove(&(old, axis, button)) {
                self.input.axis_dpad.insert((new, axis, button));
            }
        }
    }

    /// Move every `(old, key)` entry of a set to `(new, key)`.
    fn migrate_set(
        set: &mut std::collections::HashSet<(SourceId, SourceButtonId)>,
        old: SourceId,
        new: SourceId,
    ) {
        let held: Vec<SourceButtonId> = set
            .iter()
            .filter_map(|(source, button)| (*source == old).then_some(*button))
            .collect();
        for button in held {
            if set.remove(&(old, button)) {
                set.insert((new, button));
            }
        }
    }

    /// Move every `(old, key)` entry of a map to `(new, key)`, keeping values.
    fn migrate_values<K: Eq + std::hash::Hash + Copy, V: Copy>(
        map: &mut std::collections::HashMap<(SourceId, K), V>,
        old: SourceId,
        new: SourceId,
    ) {
        let stored: Vec<(K, V)> = map
            .iter()
            .filter_map(|((source, key), value)| (*source == old).then_some((*key, *value)))
            .collect();
        for (key, value) in stored {
            if map.remove(&(old, key)).is_some() {
                map.insert((new, key), value);
            }
        }
    }

    /// Disconnect-time cleanup: release the pad's holders event-fully
    /// (Kotlin synthesizes releases for its path, but the winit fallback
    /// holds never see them), then drop the id mapping. The order entry
    /// stays so a reconnect reuses the index instead of drifting.
    /// Ordering is the contract: Kotlin sends release, then removed, then
    /// (on reconnect) added, processed in order — so input for a dropped
    /// id always resolves transient, never to a stale source.
    pub fn note_pad_disconnected(&mut self, device_id: i32) {
        let guid = self.input.pads.device_ids.remove(&device_id);
        let Some(guid) = guid else {
            return;
        };
        let source = SourceId::Pad(guid);
        let held: Vec<SourceButtonId> = self
            .input
            .pressed
            .iter()
            .filter_map(|(held_source, button)| (*held_source == source).then_some(*button))
            .collect();
        for button in held {
            self.input.pressed.remove(&(source, button));
            if let SourceButtonId::Pad(std_button) = button {
                let index = self.input.pads.pad_index(source);
                // A disconnect releases like any other release: clear a
                // fired shortcut so the action stays fireable.
                if let Some(action) = self.input.pads.shortcut_map.get(&std_button).copied() {
                    self.input.shortcut_held.remove(&action);
                }
                // Slot unknown by design (mapping already dropped):
                // releases bypass the gate, so `None` still unstrands.
                self.commit_holder_press(
                    FieldSource::Pad(source, std_button),
                    std_button,
                    index,
                    None,
                    false,
                );
            }
        }
        self.input
            .analogs
            .retain(|(held_source, _), _| *held_source != source);
        let derived: Vec<(GamepadAxis, StandardButton)> = self
            .input
            .axis_dpad
            .iter()
            .filter_map(|(held_source, axis, button)| {
                (*held_source == source).then_some((*axis, *button))
            })
            .collect();
        for (axis, button) in derived {
            self.input.axis_dpad.remove(&(source, axis, button));
            let index = self.input.pads.pad_index(source);
            self.commit_holder_press(FieldSource::Axis(source, axis), button, index, None, false);
        }
    }

    /// Single entry for every physical source. Repeat suppression is one
    /// `(source, button)` set: keyboards and pads share the rule, so no
    /// adapter owns a private pressed set. Unbound pad buttons drive
    /// gameplay; bound ones resolve through the shortcut pad column.
    /// Shortcuts fire once per action across all sources (`shortcut_held`):
    /// dual delivery of one physical press (Kotlin bridge plus winit) or
    /// two inputs sharing an action must not toggle twice.
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
                if changed && let Some(&field) = self.input.keys.field_map.get(&key) {
                    // Copy ids out: no allocation on the hot path.
                    let dpad =
                        self.input.keys.source.get(&key).copied().and_then(
                            |(attachment, control)| self.input.dpad_for(attachment, control),
                        );
                    self.commit_press(field, FieldSource::Key(key), dpad, pressed);
                }

                let action = self.input.keys.shortcut_map.get(&key).copied();
                if pressed {
                    if first_press {
                        return action
                            .filter(|action| self.fire_shortcut(*action))
                            .map(Self::shortcut_kind);
                    }
                } else if let Some(action) = action {
                    self.input.shortcut_held.remove(&action);
                }
                None
            }
            SourceButtonId::Pad(std_button) => {
                if changed {
                    self.commit_pad_press(source, std_button, pressed);
                }
                // The slot gates firing only: releases clear through the
                // static map so unassigning mid-hold cannot strand an
                // action unfireable.
                let action = self.input.pads.shortcut_map.get(&std_button).copied();
                if pressed {
                    // Parked pads never fire shortcuts: without an assigned
                    // slot there is no player for the action to address, so an
                    // Off-port pad must not drive the shared pause button.
                    if first_press && self.pad_slot_for(source).is_some() {
                        return action
                            .filter(|action| self.fire_shortcut(*action))
                            .map(Self::shortcut_kind);
                    }
                } else if let Some(action) = action {
                    self.input.shortcut_held.remove(&action);
                }
                None
            }
        }
    }

    /// One fire per held action: the first source pressing an action
    /// returns it; further presses while held suppress. Releases clear
    /// through the arms above, so one physical press never toggles twice.
    fn fire_shortcut(&mut self, action: ShortcutAction) -> bool {
        self.input.shortcut_held.insert(action)
    }

    /// Pad press tail: position button to the pad's assigned console
    /// control. Off-port pads never reach here (the slot gate parks them).
    fn commit_pad_press(&mut self, source: SourceId, button: StandardButton, pressed: bool) {
        // One order-index lookup: the slot derives from it instead of
        // re-scanning `pad_order` inside the commit.
        let index = self.input.pads.pad_index(source);
        let slot = index.and_then(|i| self.slot_for_pad_index(i));
        self.commit_holder_press(
            FieldSource::Pad(source, button),
            button,
            index,
            slot,
            pressed,
        );
    }

    /// Assigned player slot for one pad order index. `None` parks the pad.
    /// Shares the gate with `pad_slot_for`; the index form lets hot paths
    /// scan `pad_order` once per event.
    fn slot_for_pad_index(&self, index: usize) -> Option<usize> {
        self.input.slot_for_index(
            index,
            &self.settings.snapshot.shared.input.gamepad.pad_ports,
        )
    }

    /// Full-press level turning a trigger into a digital press when the
    /// pad opts into shoulder doubling. Post-deadzone units. Release uses
    /// a lower band so epsilon jitter at the edge cannot flap press/release.
    const TRIGGER_PRESS: f32 = 0.9;
    const TRIGGER_RELEASE: f32 = 0.8;

    /// Corner band for stick diagonals (post-deadzone units): below it the
    /// dominant axis wins alone, so slight off-axis drift never asserts a
    /// diagonal the player did not push. Past it both axes assert together.
    const DIAGONAL_MIN: f32 = 0.5;

    /// Substitute direction for one post-deadzone axis value. Sticks stand
    /// in for dpad on digital-only consoles; triggers double as shoulders
    /// there too, else they stay analog-only. Both follow the automatic
    /// input policy (see `rebuild_input_policy`), never a per-pad toggle.
    fn substitute_button(
        axis: GamepadAxis,
        value: f32,
        trigger_alias: bool,
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
            GamepadAxis::LeftTrigger => (trigger_alias && value >= Self::TRIGGER_PRESS)
                .then_some(StandardButton::LeftShoulder),
            GamepadAxis::RightTrigger => (trigger_alias && value >= Self::TRIGGER_PRESS)
                .then_some(StandardButton::RightShoulder),
        }
    }

    /// Analog axis entry: deadzone, store, and the digital substitute
    /// (stick dpad and trigger shoulders on digital-only consoles, raw
    /// axes where the console actually drives them). The routing follows
    /// the cached input policy; only the deadzone stays per-pad.
    /// Off-port pads park silently: no store, no substitute.
    pub fn handle_source_axis(&mut self, source: SourceId, axis: GamepadAxis, value: f32) {
        // One order-index lookup per event: the slot gate and the deadzone
        // row read this instead of re-scanning `pad_order`. Routing needs
        // no lookup: the policy cache already follows the assignments.
        let index = self.input.pads.pad_index(source);
        if index.and_then(|i| self.slot_for_pad_index(i)).is_none() {
            return;
        };
        // Per-pad deadzone from the rebind snapshot: sticks vary per
        // unit, so the pad's own row tunes it. Unmapped sources fall
        // back to the default.
        let percent = index
            .and_then(|i| self.input.pad_deadzone.get(i).copied())
            .unwrap_or(DEFAULT_DEADZONE_PERCENT)
            .clamp(0, DEADZONE_MAX_PERCENT);
        let value =
            apply_deadzone(value.clamp(-1.0, 1.0), f32::from(percent) / 100.0).clamp(-1.0, 1.0);
        self.input.analogs.insert((source, axis), value);
        self.sync_axis_substitute(
            source,
            axis,
            value,
            index,
            self.input.trigger_alias,
            self.input.stick_substitute,
        );
    }

    /// Stick axis pair for joint dpad resolution, if any axis belongs to
    /// one. Sticks resolve as a pair (see `stick_pair_substitutes`):
    /// single-axis events only carry triggers.
    fn stick_pair_axes(axis: GamepadAxis) -> Option<(GamepadAxis, GamepadAxis)> {
        match axis {
            GamepadAxis::LeftStickX | GamepadAxis::LeftStickY => {
                Some((GamepadAxis::LeftStickX, GamepadAxis::LeftStickY))
            }
            GamepadAxis::RightStickX | GamepadAxis::RightStickY => {
                Some((GamepadAxis::RightStickX, GamepadAxis::RightStickY))
            }
            _ => None,
        }
    }

    /// Joint substitute for one stick: the dominant axis asserts alone
    /// below the corner band; past it both assert the diagonal. A single
    /// deflection always asserts, so sticks stay usable without deliberate
    /// corners; ties below the band keep the already-held axis.
    fn stick_pair_substitutes(x: f32, y: f32) -> (Option<StandardButton>, Option<StandardButton>) {
        use StandardButton::{DpadDown, DpadLeft, DpadRight, DpadUp};
        let wx = if x < 0.0 {
            Some(DpadLeft)
        } else if x > 0.0 {
            Some(DpadRight)
        } else {
            None
        };
        let wy = if y < 0.0 {
            Some(DpadUp)
        } else if y > 0.0 {
            Some(DpadDown)
        } else {
            None
        };
        match (wx, wy) {
            (Some(_), Some(_)) if x.abs().min(y.abs()) >= Self::DIAGONAL_MIN => (wx, wy),
            (Some(_), Some(_)) => {
                if x.abs() >= y.abs() {
                    (wx, None)
                } else {
                    (None, wy)
                }
            }
            _ => (wx, wy),
        }
    }

    /// Edge-sync one axis holder: release the stale button, press the
    /// fresh one. Holders stay distinct from button holders, so hat and
    /// stick (or shoulder and trigger) merge instead of aliasing.
    fn sync_substitute_holder(
        &mut self,
        source: SourceId,
        axis: GamepadAxis,
        want: Option<StandardButton>,
        index: Option<usize>,
        slot: Option<usize>,
    ) {
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
            self.commit_holder_press(FieldSource::Axis(source, axis), stale, index, slot, false);
        }
        if let Some(button) = want
            && self.input.axis_dpad.insert((source, axis, button))
        {
            self.commit_holder_press(FieldSource::Axis(source, axis), button, index, slot, true);
        }
    }
    /// Edge-sync axis substitutes: sticks resolve as a pair (dominant axis
    /// alone below the corner band, diagonal past it); triggers resolve
    /// alone with release-band hysteresis. Axis holders stay distinct from
    /// button holders, so hat and stick (or shoulder and trigger) merge
    /// instead of aliasing. Routing flags arrive precomputed from the
    /// automatic input policy: no per-event settings lookup.
    fn sync_axis_substitute(
        &mut self,
        source: SourceId,
        axis: GamepadAxis,
        value: f32,
        index: Option<usize>,
        trigger_alias: bool,
        sticks_as_dpad: [bool; 2],
    ) {
        let slot = index.and_then(|i| self.slot_for_pad_index(i));
        if let Some((ax, ay)) = Self::stick_pair_axes(axis) {
            // The current value is already stored: re-resolve the pair so
            // both holders converge instead of stranding on stale state.
            // A stick the console drives stays raw (the analog still
            // stores above).
            let x = self
                .input
                .analogs
                .get(&(source, ax))
                .copied()
                .unwrap_or(0.0);
            let y = self
                .input
                .analogs
                .get(&(source, ay))
                .copied()
                .unwrap_or(0.0);
            let (mut wx, mut wy) = Self::stick_pair_substitutes(x, y);
            let left = ax == GamepadAxis::LeftStickX;
            if !(if left {
                sticks_as_dpad[0]
            } else {
                sticks_as_dpad[1]
            }) {
                wx = None;
                wy = None;
            }
            self.sync_substitute_holder(source, ax, wx, index, slot);
            self.sync_substitute_holder(source, ay, wy, index, slot);
            return;
        }
        // A stick the console actually drives stays raw (the analog still
        // stores above); only substitutes park here.
        let mut want = slot.and(Self::substitute_button(axis, value, trigger_alias));
        // Trigger hysteresis: a held shoulder survives down to the release
        // band so epsilon jitter at the press edge cannot flap.
        if want.is_none()
            && trigger_alias
            && matches!(axis, GamepadAxis::LeftTrigger | GamepadAxis::RightTrigger)
            && value >= Self::TRIGGER_RELEASE
            && let Some(held) = self
                .input
                .axis_dpad
                .iter()
                .find(|(held_source, held_axis, _)| *held_source == source && *held_axis == axis)
                .map(|(_, _, button)| *button)
        {
            want = Some(held);
        }
        self.sync_substitute_holder(source, axis, want, index, slot);
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
        pad_index: Option<usize>,
        slot: Option<usize>,
        pressed: bool,
    ) {
        // Releases always travel through the OR machine: a pad parked
        // (or unplugged) mid-hold must still release its fields instead
        // of stranding them. Only fresh presses need an assigned slot.
        let Some(slot) = slot else {
            if pressed {
                return;
            }
            // Slot unknown on release: resolve the field without a slot
            // (overrides first, then the fixed layout slot-free).
            return self.commit_holder_release(holder, button, pad_index);
        };
        // A switched-off hat parks physical presses (sticks still drive
        // through their axis substitutes, which ride `Axis` holders).
        // Releases always travel below, so a mid-hold switch-off cannot
        // strand the field.
        if pressed
            && matches!(holder, FieldSource::Pad(_, _))
            && matches!(
                button,
                StandardButton::DpadUp
                    | StandardButton::DpadDown
                    | StandardButton::DpadLeft
                    | StandardButton::DpadRight
            )
            && pad_index.is_some_and(|index| self.input.pad_hat.get(index) == Some(&false))
        {
            return;
        }
        // Per-pad overrides win inside the slot: an explicit `(attachment,
        // control)` for this pad's button bypasses the standard layout.
        // The index arrives with the call so the map read never re-scans
        // `pad_order`.
        let resolved = pad_index.and_then(|index| {
            self.input
                .pads
                .binding_map
                .get(index)
                .and_then(|map| map.get(&button).copied())
        });
        if let Some((attachment, control)) = resolved {
            let Some(field) = self.resolve_field(attachment, control) else {
                log::debug!(
                    "pad override without a live field: {attachment}.{control} from {button:?}"
                );
                return;
            };
            if log::log_enabled!(log::Level::Debug) {
                log::debug!(
                    "pad override drives field {field}: {attachment}.{control} from {button:?}"
                );
            }
            let dpad = self.input.dpad_for(attachment, control);
            self.commit_press(field, holder, dpad, pressed);
            return;
        }
        let Some(abstract_key) = button.abstract_key() else {
            return;
        };
        // Routes fuse at rebind: the edge resolves here instead of
        // walking the topology per edge.
        let Some(((attachment, control), dpad)) = self
            .input
            .abstract_route
            .get(&(abstract_key, Some(slot)))
            .copied()
            .flatten()
        else {
            return;
        };
        let Some(field) = self.resolve_field(attachment, control) else {
            return;
        };
        self.commit_press(field, holder, dpad, pressed);
    }

    /// Release-only twin of the press tail: resolves without a slot so a
    /// mid-hold park or unplug cannot strand the field. Overrides first
    /// (same map), then the fixed layout scanned across every slot.
    fn commit_holder_release(
        &mut self,
        holder: FieldSource,
        button: StandardButton,
        pad_index: Option<usize>,
    ) {
        let resolved = pad_index.and_then(|index| {
            self.input
                .pads
                .binding_map
                .get(index)
                .and_then(|map| map.get(&button).copied())
        });
        if let Some((attachment, control)) = resolved {
            let Some(field) = self.resolve_field(attachment, control) else {
                return;
            };
            let dpad = self.input.dpad_for(attachment, control);
            self.commit_press(field, holder, dpad, false);
            return;
        }
        let Some(abstract_key) = button.abstract_key() else {
            return;
        };
        let Some(((attachment, control), dpad)) = self
            .input
            .abstract_route
            .get(&(abstract_key, None))
            .copied()
            .flatten()
        else {
            return;
        };
        let Some(field) = self.resolve_field(attachment, control) else {
            return;
        };
        self.commit_press(field, holder, dpad, false);
    }

    /// Field index for one `(attachment, control)`, or `None` when the
    /// live topology has no such control.
    fn resolve_field(&self, attachment: AttachmentId, control: DigitalControlId) -> Option<usize> {
        self.emu
            .field_map()
            .and_then(|fields| fields.get(&(attachment, ControlId::from(control))).copied())
    }

    /// Pad-button holds in suppression order: the shared collector for
    /// release and re-assert so the two never drift.
    fn pad_holds(
        pressed: &std::collections::HashSet<(SourceId, SourceButtonId)>,
    ) -> Vec<(SourceId, StandardButton)> {
        pressed
            .iter()
            .filter_map(|(source, button)| match button {
                SourceButtonId::Pad(std_button) => Some((*source, *std_button)),
                SourceButtonId::Key(_) => None,
            })
            .collect()
    }

    /// Event-ful release of every held pad button (surface loss): releases
    /// travel through the OR machine so fields held by other sources stay
    /// asserted. Stick-derived holds release too, and stored analogs drop
    /// (only pads ever store them; keyboards have no axis path).
    pub fn release_pad_presses(&mut self) {
        for (source, button) in Self::pad_holds(&self.input.pressed) {
            self.input
                .pressed
                .remove(&(source, SourceButtonId::Pad(button)));
            self.commit_pad_press(source, button, false);
        }
        // Full release like any other: a shortcut held across the loss
        // must fire again on the next press instead of staying suppressed.
        self.input.shortcut_held.clear();
        let derived: Vec<(SourceId, GamepadAxis, StandardButton)> =
            self.input.axis_dpad.iter().copied().collect();
        for (source, axis, button) in derived {
            self.input.axis_dpad.remove(&(source, axis, button));
            let index = self.input.pads.pad_index(source);
            let slot = index.and_then(|i| self.slot_for_pad_index(i));
            self.commit_holder_press(FieldSource::Axis(source, axis), button, index, slot, false);
        }
        self.input.analogs.clear();
    }

    /// Re-assert every held pad button through the live assignments.
    /// Pair with the keyboard re-assert in `rebind_input`: the live
    /// fields were just cleared, so both sources survive a settings
    /// apply without a re-press. Pads re-resolve (ports may have moved);
    /// Off-parked holds simply assert nothing.
    pub(super) fn reassert_pad_presses(&mut self) {
        let held = Self::pad_holds(&self.input.pressed);
        for (source, button) in held {
            self.commit_pad_press(source, button, true);
        }
    }

    pub fn clear_input(&mut self) {
        self.input.pressed.clear();
        self.input.analogs.clear();
        self.input.axis_dpad.clear();
        self.input.shortcut_held.clear();
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
        // profile is active. The stick/trigger policy rebuilds here too:
        // it follows the assignments, so a system switch re-routes
        // without any per-pad setting.
        self.input.rebuild_key_field_map(
            bindings.map(|p| p.bindings.as_slice()),
            self.emu.field_map(),
        );
        self.input.rebuild_pad_binding_map(pad_bindings);
        self.input.rebuild_input_policy();
        self.input
            .rebuild_pad_params(&self.settings.snapshot.shared.input.gamepad);
        self.input
            .rebuild_shortcut_maps(&self.settings.snapshot.shared.input.shortcuts.keyboard);
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

    /// Analog console mock: one stick axis, so the input policy parks
    /// substitutes and trigger doubling instead of driving the dpad.
    #[derive(Debug)]
    struct MockAnalogProfile;

    impl ControllerProfile for MockAnalogProfile {
        fn profile_id(&self) -> ProfileId {
            ProfileId::new("test.analog")
        }

        fn label(&self) -> &'static str {
            "Analog Pad"
        }

        fn port_sets(&self) -> &[PortSet] {
            static PORTS: &[AttachmentId] = &[AttachmentId::new("test.topology.slot")];
            static SETS: &[PortSet] = &[PortSet { ports: PORTS }];
            SETS
        }

        fn port_groups(&self) -> &[&[ControlInfo]] {
            static CONTROLS: &[ControlInfo] = &[ControlInfo {
                id: ControlId::digital(DigitalControlId::new("test.topology.stick")),
                label: "Stick",
                kind: ControlKind::Analog,
                abstract_key: Some(AbstractKey::Axis1X),
            }];
            static GROUPS: &[&[ControlInfo]] = &[CONTROLS];
            GROUPS
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
        session.note_pad_connected(11, "test-pad-11");
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
    fn one_physical_press_fires_its_shortcut_once_across_sources() {
        use nerust_input_traits::{SourceButtonId, StandardButton};
        use nerust_settings_core::model::input::{ShortcutAction, ShortcutBinding};
        // Dual delivery (Kotlin bridge plus winit) of one physical press,
        // or two inputs sharing an action, must not toggle twice: the
        // action fires once while held, from whichever source arrives
        // first. Either trigger, not both together.
        let mut session = crate::session::test_util::test_session();
        let mut next = session.settings_snapshot().clone();
        next.shared.input.shortcuts.keyboard = vec![ShortcutBinding {
            action: ShortcutAction::TogglePause,
            key: Some(Key::Space),
            pad: Some(StandardButton::Guide),
        }];
        session.apply_settings(next).unwrap();
        session.note_pad_connected(11, "test-pad-11");
        let pad = session.pad_source_for(11);
        assert!(
            session.handle_keyboard_key(Key::Space, true).is_some(),
            "first source fires"
        );
        assert!(
            session
                .handle_source_button(pad, SourceButtonId::Pad(StandardButton::Guide), true)
                .is_none(),
            "second source suppresses while held"
        );
        session.handle_source_button(pad, SourceButtonId::Pad(StandardButton::Guide), false);
        session.handle_keyboard_key(Key::Space, false);
        assert!(
            session
                .handle_source_button(pad, SourceButtonId::Pad(StandardButton::Guide), true)
                .is_some(),
            "release re-arms the action"
        );
        session.clear_input();
    }

    #[test]
    fn focus_loss_release_re_arms_shortcuts() {
        // A shortcut held across focus loss must fire again on the next
        // press: the bulk release clears the held action like any release.
        use nerust_input_traits::{SourceButtonId, StandardButton};
        use nerust_settings_core::model::input::{ShortcutAction, ShortcutBinding};
        let mut session = crate::session::test_util::test_session();
        let mut next = session.settings_snapshot().clone();
        next.shared.input.shortcuts.keyboard = vec![ShortcutBinding {
            action: ShortcutAction::TogglePause,
            key: None,
            pad: Some(StandardButton::Guide),
        }];
        session.apply_settings(next).unwrap();
        session.note_pad_connected(11, "test-pad-11");
        let pad = session.pad_source_for(11);
        assert!(
            session
                .handle_source_button(pad, SourceButtonId::Pad(StandardButton::Guide), true)
                .is_some(),
            "first press fires"
        );
        session.release_pad_presses();
        assert!(
            session
                .handle_source_button(pad, SourceButtonId::Pad(StandardButton::Guide), true)
                .is_some(),
            "post-loss press fires instead of staying suppressed"
        );
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
                .pads
                .binding_map
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
        let map = &session.input.pads.binding_map[0];
        assert_eq!(map.len(), 1, "analog entries never reach the map");
        assert_eq!(
            map[&StandardButton::East],
            (
                AttachmentId::new("test.topology.slot"),
                DigitalControlId::new("test.topology.a")
            )
        );
        assert!(
            session.input.pads.binding_map[1].is_empty(),
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
        let map = &session.input.pads.binding_map[0];
        assert_eq!(map.len(), 1);
        assert_eq!(
            map[&StandardButton::East].1,
            DigitalControlId::new("test.topology.a"),
            "first entry wins, the duplicate only logs"
        );
    }

    #[test]
    fn pad_override_swap_drives_remapped_fields() {
        // A/B swap on a pad screen must reach the game: South drives
        // control b and East drives control a after the swap.
        use nerust_input_traits::{ControlId, SourceButtonId, StandardButton};
        use nerust_settings_core::model::input::{
            PadBinding, PersistedControlId, SystemInputSettings,
        };
        let mut session = crate::session::test_util::test_session();
        session.input.current_assignments = InputAssignments::new(vec![(
            AttachmentId::new("test.topology.slot"),
            Some(Rc::new(MockTopologyProfile)),
        )]);
        let mut next = session.settings_snapshot().clone();
        let system = session.active_system_id_box().expect("test system active");
        let mut system_settings = SystemInputSettings::default();
        system_settings.pad_bindings[0] = vec![
            PadBinding::new(
                "test.topology.slot",
                PersistedControlId::digital("test.topology.b"),
                StandardButton::South,
            ),
            PadBinding::new(
                "test.topology.slot",
                PersistedControlId::digital("test.topology.a"),
                StandardButton::East,
            ),
        ];
        next.shared.input.systems.insert(system, system_settings);
        session.apply_settings(next).unwrap();
        // Observable runtime: the mock core contributes no field map, so
        // install one for the swapped controls.
        let attachment = AttachmentId::new("test.topology.slot");
        let mut parts = crate::test_helpers::build_test_core_parts();
        let _ = parts.field_map.insert(
            (
                attachment,
                ControlId::digital(DigitalControlId::new("test.topology.a")),
            ),
            1,
        );
        let _ = parts.field_map.insert(
            (
                attachment,
                ControlId::digital(DigitalControlId::new("test.topology.b")),
            ),
            2,
        );
        let (emu_core, gui_input, field_map, host_peripherals) =
            crate::emu_core::EmuCore::from_parts(parts);
        session.emu.runtime = Some(crate::session::ActiveRuntime {
            core: Box::new(emu_core),
            input: gui_input,
            field_map,
            host_peripherals,
        });
        session.note_pad_connected(11, "test-pad-11");
        let pad = session.pad_source_for(11);
        session.handle_source_button(pad, SourceButtonId::Pad(StandardButton::South), true);
        assert_eq!(
            session.input.press_state.asserted_fields(),
            vec![2],
            "swapped South drives control b, not a"
        );
        session.handle_source_button(pad, SourceButtonId::Pad(StandardButton::South), false);
        session.handle_source_button(pad, SourceButtonId::Pad(StandardButton::East), true);
        assert_eq!(
            session.input.press_state.asserted_fields(),
            vec![1],
            "swapped East drives control a, not b"
        );
        session.handle_source_button(pad, SourceButtonId::Pad(StandardButton::East), false);
        assert!(
            session.input.press_state.asserted_fields().is_empty(),
            "releases unassert"
        );
        session.clear_input();
    }

    #[test]
    fn stray_pre_connect_key_never_shifts_a_swap_onto_the_wrong_pad() {
        // Third-report regression (GBA A/B swap showed in settings but not
        // in game): a stray pre-connect key from another device id must
        // neither drive pad 0 nor shift the real pad's overrides. The UI
        // sorts by the descriptor mirror (pad 0 == the real pad), so the
        // commit path must resolve the same rank.
        use nerust_input_traits::{ControlId, SourceButtonId, StandardButton};
        use nerust_settings_core::model::input::{
            PadBinding, PersistedControlId, SystemInputSettings,
        };
        let mut session = crate::session::test_util::test_session();
        session.input.current_assignments = InputAssignments::new(vec![(
            AttachmentId::new("test.topology.slot"),
            Some(Rc::new(MockTopologyProfile)),
        )]);
        // Stray key from a device that never connects: parks, reserves
        // nothing, and never appears in the descriptor mirror.
        let stray = session.pad_source_for(77);
        session.handle_source_button(stray, SourceButtonId::Pad(StandardButton::South), true);
        assert!(session.input.press_state.asserted_fields().is_empty());
        assert!(session.pad_descriptors().is_empty());
        // The real pad connects after the stray: UI pad 0, setting rank 0.
        session.note_pad_connected(11, "test-pad-11");
        let pad = session.pad_source_for(11);
        assert_eq!(session.pad_descriptors(), &["test-pad-11"]);
        // A/B swap stored at UI pad 0 (the vault index).
        let mut next = session.settings_snapshot().clone();
        let system = session.active_system_id_box().expect("test system active");
        let mut system_settings = SystemInputSettings::default();
        system_settings.pad_bindings[0] = vec![
            PadBinding::new(
                "test.topology.slot",
                PersistedControlId::digital("test.topology.b"),
                StandardButton::South,
            ),
            PadBinding::new(
                "test.topology.slot",
                PersistedControlId::digital("test.topology.a"),
                StandardButton::East,
            ),
        ];
        next.shared.input.systems.insert(system, system_settings);
        session.apply_settings(next).unwrap();
        let attachment = AttachmentId::new("test.topology.slot");
        let mut parts = crate::test_helpers::build_test_core_parts();
        let _ = parts.field_map.insert(
            (
                attachment,
                ControlId::digital(DigitalControlId::new("test.topology.a")),
            ),
            1,
        );
        let _ = parts.field_map.insert(
            (
                attachment,
                ControlId::digital(DigitalControlId::new("test.topology.b")),
            ),
            2,
        );
        let (emu_core, gui_input, field_map, host_peripherals) =
            crate::emu_core::EmuCore::from_parts(parts);
        session.emu.runtime = Some(crate::session::ActiveRuntime {
            core: Box::new(emu_core),
            input: gui_input,
            field_map,
            host_peripherals,
        });
        session.handle_source_button(pad, SourceButtonId::Pad(StandardButton::South), true);
        assert_eq!(
            session.input.press_state.asserted_fields(),
            vec![2],
            "stray key must not shift the swap: South still drives b"
        );
        session.handle_source_button(pad, SourceButtonId::Pad(StandardButton::South), false);
        session.handle_source_button(pad, SourceButtonId::Pad(StandardButton::East), true);
        assert_eq!(
            session.input.press_state.asserted_fields(),
            vec![1],
            "stray key must not shift the swap: East still drives a"
        );
        session.handle_source_button(pad, SourceButtonId::Pad(StandardButton::East), false);
        session.clear_input();
    }

    #[test]
    fn axis_deadzone_reads_per_pad_rows() {
        use nerust_input_traits::GamepadAxis;
        let mut session = crate::session::test_util::test_session();
        session.note_pad_connected(11, "test-pad-11");
        session.note_pad_connected(12, "test-pad-12");
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
        session.note_pad_connected(11, "test-pad-11");
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
                - apply_deadzone(
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
    fn shallow_diagonal_asserts_dominant_axis_only() {
        use nerust_input_traits::{GamepadAxis, StandardButton};
        let mut session = crate::session::test_util::test_session();
        session.input.current_assignments = InputAssignments::new(vec![(
            AttachmentId::new("test.topology.slot"),
            Some(Rc::new(MockTopologyProfile)),
        )]);
        let mut next = session.settings_snapshot().clone();
        next.shared.input.gamepad.pad_ports[0].deadzone_percent = 0;
        session.apply_settings(next).unwrap();
        session.note_pad_connected(11, "test-pad-11");
        let pad = session.pad_source_for(11);
        let held = |session: &SessionHandle, axis: GamepadAxis| {
            session
                .input
                .axis_dpad
                .iter()
                .filter(|(source, held_axis, _)| *source == pad && *held_axis == axis)
                .map(|(_, _, button)| *button)
                .collect::<Vec<_>>()
        };
        // Hard right first: only Right asserts.
        session.handle_source_axis(pad, GamepadAxis::LeftStickX, 0.8);
        assert_eq!(
            held(&session, GamepadAxis::LeftStickX),
            [StandardButton::DpadRight]
        );
        // Slight down stays a Right: the corner band (0.5) is not met and
        // X dominates, so no diagonal fires.
        session.handle_source_axis(pad, GamepadAxis::LeftStickY, 0.3);
        assert_eq!(
            held(&session, GamepadAxis::LeftStickX),
            [StandardButton::DpadRight]
        );
        assert!(held(&session, GamepadAxis::LeftStickY).is_empty());
        // Deliberate corner: past the band both assert.
        session.handle_source_axis(pad, GamepadAxis::LeftStickY, 0.6);
        assert_eq!(
            held(&session, GamepadAxis::LeftStickX),
            [StandardButton::DpadRight]
        );
        assert_eq!(
            held(&session, GamepadAxis::LeftStickY),
            [StandardButton::DpadDown]
        );
        // X eases off: Right releases, Down holds alone.
        session.handle_source_axis(pad, GamepadAxis::LeftStickX, 0.2);
        assert!(held(&session, GamepadAxis::LeftStickX).is_empty());
        assert_eq!(
            held(&session, GamepadAxis::LeftStickY),
            [StandardButton::DpadDown]
        );
        session.clear_input();
    }

    #[test]
    fn off_port_pads_park_silently() {
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
            !session
                .input
                .analogs
                .contains_key(&(pad, GamepadAxis::LeftStickX)),
            "parked pads store nothing"
        );
        assert!(session.input.axis_dpad.is_empty());
        session.clear_input();
    }

    #[test]
    fn primary_pad_source_follows_first_seen_pad() {
        let mut session = crate::session::test_util::test_session();
        assert!(
            session.primary_pad_source().is_none(),
            "no pads seen yet: the caller must drop the keys"
        );
        session.note_pad_connected(11, "test-pad-11");
        let pad = session.pad_source_for(11);
        assert_eq!(session.primary_pad_source(), Some(pad));
        session.clear_input();
    }

    #[test]
    fn parked_pads_do_not_fire_shortcuts() {
        use nerust_input_traits::{SourceButtonId, StandardButton};
        use nerust_settings_core::model::input::ShortcutAction;
        let mut session = crate::session::test_util::test_session();
        let mut next = session.settings_snapshot().clone();
        next.shared.input.gamepad.pad_ports = Default::default();
        next.shared.input.shortcuts.keyboard =
            vec![nerust_settings_core::model::input::ShortcutBinding {
                action: ShortcutAction::TogglePause,
                key: None,
                pad: Some(StandardButton::Guide),
            }];
        session.apply_settings(next).unwrap();
        let pad = session.pad_source_for(11);
        assert!(
            session
                .handle_source_button(pad, SourceButtonId::Pad(StandardButton::Guide), true)
                .is_none(),
            "parked pads never fire shortcuts"
        );
        session.clear_input();
    }

    #[test]
    fn connect_migrates_holders_to_descriptor_source() {
        use nerust_input_traits::{GamepadAxis, SourceButtonId, StandardButton};
        let mut session = crate::session::test_util::test_session();
        session.input.current_assignments = InputAssignments::new(vec![(
            AttachmentId::new("test.topology.slot"),
            Some(Rc::new(MockTopologyProfile)),
        )]);
        // Input before the connect event parks on a transient key: the
        // hold records (so the release below still unstrands) but drives
        // no field and stores no analog until the descriptor arrives.
        let transient = session.pad_source_for(7);
        session.handle_source_button(transient, SourceButtonId::Pad(StandardButton::South), true);
        session.handle_source_axis(transient, GamepadAxis::LeftStickX, 0.8);
        assert!(
            session
                .input
                .pressed
                .contains(&(transient, SourceButtonId::Pad(StandardButton::South)))
        );
        assert!(
            !session
                .input
                .analogs
                .contains_key(&(transient, GamepadAxis::LeftStickX)),
            "pre-connect axes park instead of driving pad 0"
        );
        // The connect collapses in place: holders migrate to the
        // descriptor source instead of stranding on the dead key.
        session.note_pad_connected(7, "pad-desc-A");
        let descriptor = session.pad_source_for(7);
        assert_ne!(transient, descriptor);
        assert!(
            session
                .input
                .pressed
                .contains(&(descriptor, SourceButtonId::Pad(StandardButton::South)))
        );
        // The parked pre-connect axis stored nothing, so nothing migrates;
        // post-connect input stores under the descriptor source.
        session.handle_source_axis(descriptor, GamepadAxis::LeftStickX, 0.8);
        assert!(
            session
                .input
                .analogs
                .contains_key(&(descriptor, GamepadAxis::LeftStickX))
        );
        assert!(session.input.axis_dpad.contains(&(
            descriptor,
            GamepadAxis::LeftStickX,
            StandardButton::DpadRight
        )));
        assert!(
            !session
                .input
                .pressed
                .iter()
                .any(|(source, _)| *source == transient)
        );
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
                .pads
                .order
                .contains(&crate::session::PadTopology::guid_for_transient(7))
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
    fn descriptor_mirror_tracks_collapse_slots() {
        let mut session = crate::session::test_util::test_session();
        // Interleaved transient input and connects: the mirror must name
        // the same slots as the session order, or the vault display
        // skews onto another pad's settings.
        let _ = session.pad_source_for(7);
        session.note_pad_connected(9, "pad-desc-B");
        session.note_pad_connected(7, "pad-desc-A");
        assert_eq!(session.pad_descriptors(), &["pad-desc-A", "pad-desc-B"]);
        assert_eq!(
            session.input.pads.order.len(),
            session.pad_descriptors().len(),
            "no transients leak into the mirror"
        );
        // Reconnect reuses the slot; the mirror never moves it.
        session.note_pad_disconnected(7);
        session.note_pad_connected(11, "pad-desc-A");
        assert_eq!(session.pad_descriptors(), &["pad-desc-A", "pad-desc-B"]);
        session.clear_input();
    }

    #[test]
    fn release_while_parked_still_unstrands() {
        use nerust_input_traits::{SourceButtonId, StandardButton};
        let mut session = crate::session::test_util::test_session();
        let pad = session.pad_source_for(11);
        session.handle_source_button(pad, SourceButtonId::Pad(StandardButton::South), true);
        // Park mid-hold: the release must still travel (no slot gate).
        let mut next = session.settings_snapshot().clone();
        next.shared.input.gamepad.pad_ports = Default::default();
        session.apply_settings(next).unwrap();
        session.handle_source_button(pad, SourceButtonId::Pad(StandardButton::South), false);
        session.clear_input();
    }

    #[test]
    fn disconnect_releases_winit_holds() {
        use nerust_input_traits::{SourceButtonId, StandardButton};
        let mut session = crate::session::test_util::test_session();
        session.note_pad_connected(5, "pad-disconnect-test");
        let pad = session.pad_source_for(5);
        session.handle_source_button(pad, SourceButtonId::Pad(StandardButton::South), true);
        session.note_pad_disconnected(5);
        assert!(
            !session
                .input
                .pressed
                .iter()
                .any(|(source, _)| *source == pad),
            "disconnect drops the pressed hold"
        );
        session.clear_input();
    }

    #[test]
    fn primary_pad_prefers_assigned_over_first_seen() {
        let mut session = crate::session::test_util::test_session();
        session.note_pad_connected(7, "pad-first-parked");
        session.note_pad_connected(8, "pad-second-assigned");
        let mut next = session.settings_snapshot().clone();
        next.shared.input.gamepad.pad_ports[0].slot = None;
        next.shared.input.gamepad.pad_ports[1].slot = Some(0);
        session.apply_settings(next).unwrap();
        let second = session.pad_source_for(8);
        assert_eq!(session.primary_pad_source(), Some(second));
        session.clear_input();
    }

    #[test]
    fn negative_device_ids_never_consume_order() {
        let mut session = crate::session::test_util::test_session();
        let before = session.input.pads.order.len();
        let _ = session.pad_source_for(-1);
        assert_eq!(session.input.pads.order.len(), before);
        session.note_pad_connected(-1, "ghost");
        assert_eq!(session.input.pads.order.len(), before);
        session.clear_input();
    }

    #[test]
    fn trigger_hysteresis_holds_through_the_release_band() {
        use nerust_input_traits::{GamepadAxis, StandardButton};
        let mut session = crate::session::test_util::test_session();
        session.note_pad_connected(9, "pad-trigger-test");
        let pad = session.pad_source_for(9);
        let mut next = session.settings_snapshot().clone();
        next.shared.input.gamepad.pad_ports[0].slot = Some(0);
        session.apply_settings(next).unwrap();
        session.handle_source_axis(pad, GamepadAxis::LeftTrigger, 0.95);
        assert!(
            session.input.axis_dpad.contains(&(
                pad,
                GamepadAxis::LeftTrigger,
                StandardButton::LeftShoulder
            )),
            "full press engages the shoulder"
        );
        session.handle_source_axis(pad, GamepadAxis::LeftTrigger, 0.85);
        assert!(
            session.input.axis_dpad.contains(&(
                pad,
                GamepadAxis::LeftTrigger,
                StandardButton::LeftShoulder
            )),
            "release band holds instead of flapping"
        );
        session.handle_source_axis(pad, GamepadAxis::LeftTrigger, 0.5);
        assert!(
            !session
                .input
                .axis_dpad
                .iter()
                .any(|(source, axis, _)| *source == pad && *axis == GamepadAxis::LeftTrigger),
            "below the band releases"
        );
        session.clear_input();
    }

    #[test]
    fn routing_follows_the_console_not_a_toggle() {
        // Digital console: triggers double as shoulders and sticks
        // substitute the dpad, with no opt-in anywhere.
        use nerust_input_traits::{GamepadAxis, StandardButton};
        let mut session = crate::session::test_util::test_session();
        session.input.current_assignments = InputAssignments::new(vec![(
            AttachmentId::new("test.topology.slot"),
            Some(Rc::new(MockTopologyProfile)),
        )]);
        session.note_pad_connected(11, "test-pad-11");
        let pad = session.pad_source_for(11);
        session.handle_source_axis(pad, GamepadAxis::LeftTrigger, 1.0);
        assert!(session.input.axis_dpad.contains(&(
            pad,
            GamepadAxis::LeftTrigger,
            StandardButton::LeftShoulder
        )));
        session.handle_source_axis(pad, GamepadAxis::LeftTrigger, 0.0);
        assert!(session.input.axis_dpad.is_empty());
        session.handle_source_axis(pad, GamepadAxis::LeftStickX, 0.8);
        assert!(session.input.axis_dpad.contains(&(
            pad,
            GamepadAxis::LeftStickX,
            StandardButton::DpadRight
        )));
        session.clear_input();
        // Analog console: the same inputs store raw and assert nothing.
        session.input.current_assignments = InputAssignments::new(vec![(
            AttachmentId::new("test.topology.slot"),
            Some(Rc::new(MockAnalogProfile)),
        )]);
        session.rebuild_key_field_map();
        session.handle_source_axis(pad, GamepadAxis::LeftTrigger, 1.0);
        assert!(
            session
                .input
                .analogs
                .contains_key(&(pad, GamepadAxis::LeftTrigger)),
            "analog consoles still record the pull"
        );
        assert!(
            session.input.axis_dpad.is_empty(),
            "no shoulder doubling where triggers stay analog"
        );
        session.handle_source_axis(pad, GamepadAxis::LeftStickX, 0.8);
        assert!(
            session
                .input
                .axis_dpad
                .iter()
                .all(|(source, axis, _)| { *source != pad || *axis != GamepadAxis::LeftStickX }),
            "no dpad substitution where the console drives the stick"
        );
        session.clear_input();
    }

    #[test]
    fn analog_console_parks_substitutes_but_still_stores_axes() {
        use nerust_input_traits::{GamepadAxis, StandardButton};
        let mut session = crate::session::test_util::test_session();
        session.input.current_assignments = InputAssignments::new(vec![(
            AttachmentId::new("test.topology.slot"),
            Some(Rc::new(MockTopologyProfile)),
        )]);
        session.note_pad_connected(11, "test-pad-11");
        let pad = session.pad_source_for(11);
        // Digital console: both sticks substitute.
        session.handle_source_axis(pad, GamepadAxis::LeftStickX, 0.8);
        assert!(session.input.axis_dpad.contains(&(
            pad,
            GamepadAxis::LeftStickX,
            StandardButton::DpadRight
        )));
        session.clear_input();
        // Analog console (drives the left stick): the substitute parks
        // while the analog value still stores; the right stick keeps
        // substituting.
        session.input.current_assignments = InputAssignments::new(vec![(
            AttachmentId::new("test.topology.slot"),
            Some(Rc::new(MockAnalogProfile)),
        )]);
        session.rebuild_key_field_map();
        session.handle_source_axis(pad, GamepadAxis::LeftStickX, 0.8);
        assert!(
            session
                .input
                .analogs
                .contains_key(&(pad, GamepadAxis::LeftStickX)),
            "analog consoles still record the deflection"
        );
        assert!(
            !session
                .input
                .axis_dpad
                .iter()
                .any(|(source, axis, _)| *source == pad && *axis == GamepadAxis::LeftStickX),
            "consumed stick asserts no dpad"
        );
        session.handle_source_axis(pad, GamepadAxis::RightStickX, 0.8);
        assert!(session.input.axis_dpad.contains(&(
            pad,
            GamepadAxis::RightStickX,
            StandardButton::DpadRight
        )));
        session.clear_input();
    }

    #[test]
    fn switched_off_hat_parks_physical_presses_but_not_stick_substitutes() {
        use nerust_input_traits::{ControlId, GamepadAxis, SourceButtonId, StandardButton};
        use nerust_settings_core::model::input::{
            PadBinding, PersistedControlId, SystemInputSettings,
        };
        let mut session = crate::session::test_util::test_session();
        session.input.current_assignments = InputAssignments::new(vec![(
            AttachmentId::new("test.topology.slot"),
            Some(Rc::new(MockTopologyProfile)),
        )]);
        session.note_pad_connected(11, "test-pad-11");
        let pad = session.pad_source_for(11);
        // Override parking the hat on a live field, so the gate (not a
        // missing field) decides.
        let mut next = session.settings_snapshot().clone();
        let system = session.active_system_id_box().expect("test system active");
        let mut system_settings = SystemInputSettings::default();
        system_settings.pad_bindings[0] = vec![PadBinding::new(
            "test.topology.slot",
            PersistedControlId::digital("test.topology.a"),
            StandardButton::DpadUp,
        )];
        next.shared.input.systems.insert(system, system_settings);
        session.apply_settings(next).unwrap();
        let attachment = AttachmentId::new("test.topology.slot");
        let mut parts = crate::test_helpers::build_test_core_parts();
        let _ = parts.field_map.insert(
            (
                attachment,
                ControlId::digital(DigitalControlId::new("test.topology.a")),
            ),
            7,
        );
        let (emu_core, gui_input, field_map, host_peripherals) =
            crate::emu_core::EmuCore::from_parts(parts);
        session.emu.runtime = Some(crate::session::ActiveRuntime {
            core: Box::new(emu_core),
            input: gui_input,
            field_map,
            host_peripherals,
        });
        // Hat on: override drives the field.
        session.handle_source_button(pad, SourceButtonId::Pad(StandardButton::DpadUp), true);
        assert_eq!(session.input.press_state.asserted_fields(), vec![7]);
        session.handle_source_button(pad, SourceButtonId::Pad(StandardButton::DpadUp), false);
        // Hat off: the same press parks, the release unstrands cleanly,
        // and the stick substitute still drives through its axis holder.
        let mut next = session.settings_snapshot().clone();
        next.shared.input.gamepad.pad_ports[0].dpad_enabled = false;
        session.apply_settings(next).unwrap();
        session.handle_source_button(pad, SourceButtonId::Pad(StandardButton::DpadUp), true);
        assert!(
            session.input.press_state.asserted_fields().is_empty(),
            "switched-off hat asserts nothing"
        );
        session.handle_source_button(pad, SourceButtonId::Pad(StandardButton::DpadUp), false);
        session.handle_source_axis(pad, GamepadAxis::LeftStickX, 0.8);
        assert!(
            session.input.axis_dpad.contains(&(
                pad,
                GamepadAxis::LeftStickX,
                StandardButton::DpadRight
            )),
            "stick substitutes ignore the hat switch"
        );
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
        use nerust_core_traits::touch::TouchControlRole;
        assert_eq!(
            TouchControlRole::for_abstract(AbstractKey::Button5),
            Some(TouchControlRole::LeftShoulder)
        );
        assert_eq!(
            TouchControlRole::for_abstract(AbstractKey::Button6),
            Some(TouchControlRole::RightShoulder)
        );
        assert_eq!(
            TouchControlRole::for_abstract(AbstractKey::Button1),
            Some(TouchControlRole::FaceButton1)
        );
        assert_eq!(TouchControlRole::for_abstract(AbstractKey::Guide), None);
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
            assert_eq!(TouchControlRole::for_abstract(key), Some(role));
        }
        assert_eq!(TouchControlRole::for_abstract(AbstractKey::Guide), None);
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

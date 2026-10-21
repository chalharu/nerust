use nerust_core_traits::touch::{TouchControl, TouchControlRole, TouchOverlayModel};
use nerust_input_traits::{AbstractKey, ControlId, DigitalInputEvent, InputValue};
use nerust_keyboard::Key;
use nerust_settings_core::model::input::ShortcutAction;

use crate::{
    session::{KeyboardShortcut, SessionHandle},
    settings::bindings::events::shortcut::shortcut_action_for_key,
};

pub use nerust_settings_core::input::clear_multi_port_conflicts;

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
                    label: info.label.to_string(),
                });
            }
        }
    }
    TouchOverlayModel { revision, controls }
}

impl SessionHandle {
    /// Called by touch overlay (Android) with a pre-resolved DigitalInputEvent.
    pub fn apply_input_event(&mut self, event: DigitalInputEvent) {
        if let Some(&field) = self
            .emu
            .field_map()
            .and_then(|fields| fields.get(&(event.attachment, ControlId::from(event.control))))
            && let Some(gui_input) = self.emu.input_mut()
        {
            let _ = gui_input.set(field, InputValue::Digital(event.is_pressed()));
        }
    }

    pub fn handle_keyboard_key(&mut self, key: Key, pressed: bool) -> Option<KeyboardShortcut> {
        let first_press = if pressed {
            self.input.pressed_keys.insert(key)
        } else {
            self.input.pressed_keys.remove(&key);
            false
        };

        if let Some(&field) = self.input.key_field_map.get(&key)
            && let Some(gui_input) = self.emu.input_mut()
        {
            let _ = gui_input.set(field, InputValue::Digital(pressed));
        }

        if first_press {
            return shortcut_action_for_key(&self.settings.snapshot.shared, key).map(|action| {
                if matches!(action, ShortcutAction::ToggleFullscreen) {
                    KeyboardShortcut::ToggleFullscreen
                } else if matches!(action, ShortcutAction::OpenDebugger) {
                    KeyboardShortcut::OpenDebugger
                } else {
                    KeyboardShortcut::Session(action)
                }
            });
        }
        None
    }

    pub fn clear_input(&mut self) {
        self.input.pressed_keys.clear();
        if let Some(gui_input) = self.emu.input_mut() {
            gui_input.clear();
        }
    }

    pub fn rebuild_key_field_map(&mut self) {
        let bindings = self.active_system_id_box().and_then(|system_id| {
            self.settings
                .snapshot
                .shared
                .input
                .systems
                .get(&system_id)
                .and_then(|s| s.implicit_keyboard_profile())
        });
        // The department owns the map; the session only resolves which
        // profile is active.
        self.input.rebuild_key_field_map(
            bindings.map(|p| p.bindings.as_slice()),
            self.emu.field_map(),
        );
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, rc::Rc};

    use nerust_gui_viewmodel::settings::{build_topology, device_kind};
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
        field_map.insert((attachment, control), 7);
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
            let fields = entries.into_iter().collect();
            let mut result = HashMap::new();
            let error = rebuild_input_map(&fields, &bindings, &mut result).unwrap_err();
            assert_eq!(error.control, "test.b");
            assert_eq!(result, HashMap::from([(Key::KeyA, 1)]));
        }
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
        assert!(session.input.pressed_keys.contains(&Key::KeyA));
        session.handle_keyboard_key(Key::KeyA, false);
        assert!(!session.input.pressed_keys.contains(&Key::KeyA));
    }

    #[test]
    fn rebuild_input_map_reports_first_duplicate_for_caller_to_log() {
        use crate::session::input_map::DuplicateBinding;
        let attachment = AttachmentId::new("test.topology.slot");
        let control = ControlId::digital(DigitalControlId::new("test.topology.a"));
        let mut field_map = UnifiedFieldMap::new();
        field_map.insert((attachment, control), 7);
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
        assert_eq!(overlay.controls[0].label, "A");

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

    #[test]
    fn device_kind_delegates_to_profile_method() {
        let profile = MockSinglePort;
        let kind = device_kind(&profile, 0);
        assert_eq!(kind.as_str(), "test.standard");
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

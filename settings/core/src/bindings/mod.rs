pub mod descriptors;

use std::collections::BTreeMap;

use crate::model::input::KeyboardProfile;
use crate::model::shared::DesktopSharedSettings;
use nerust_core_traits::identity::SystemId;
use nerust_input_traits::InputTopologyDescriptor;
use nerust_keyboard::Key;

/// Precomputed (attachment, control) → key lookup for one profile.
/// Build once per pass; repeated validations reuse it instead of re-indexing.
#[derive(Debug, Default)]
pub struct BindingIndex<'a> {
    map: std::collections::HashMap<(&'a str, &'a str), Key>,
}

impl<'a> BindingIndex<'a> {
    /// Index one profile's bindings; duplicate pairs keep the first key.
    pub fn for_profile(profile: &'a KeyboardProfile) -> Self {
        let mut map = std::collections::HashMap::with_capacity(profile.bindings.len());
        for binding in &profile.bindings {
            map.entry((binding.attachment.as_str(), binding.control.as_str()))
                .or_insert(binding.key);
        }
        Self { map }
    }

    /// Probe one descriptor pair in O(1).
    pub fn key_for(&self, attachment: &str, control: &str) -> Option<Key> {
        self.map.get(&(attachment, control)).copied()
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

/// Key conflicts with a precomputed profile index; same result as [`conflicting_keys`].
pub fn conflicting_keys_indexed(
    settings: &DesktopSharedSettings,
    topology: &InputTopologyDescriptor,
    system: &dyn SystemId,
    index: &BindingIndex<'_>,
) -> BTreeMap<Key, Vec<String>> {
    let mut by_key = BTreeMap::<Key, Vec<(&'static str, &'static str)>>::new();

    if settings
        .input
        .systems
        .get(system)
        .and_then(|s| s.implicit_keyboard_profile())
        .is_some()
    {
        for descriptor in descriptors::keyboard_binding_descriptors(topology, system) {
            let query = (descriptor.attachment.as_str(), descriptor.control.as_str());
            if let Some(key) = index.key_for(query.0, query.1) {
                by_key
                    .entry(key)
                    .or_default()
                    .push((descriptor.attachment_label, descriptor.control_label));
            }
        }
    }

    collect_shortcut_conflicts(settings, by_key)
}

/// Find key binding conflicts within a single system. Only the implicit keyboard profile is
/// checked; same-key shortcut conflicts included. Bindings are indexed once (attachment, control) → key, so the descriptor walk probes O(1) instead of scanning per descriptor; labels stay interned `&'static str` pairs until the final materialization (no `format!`).
pub fn conflicting_keys(
    settings: &DesktopSharedSettings,
    topology: &InputTopologyDescriptor,
    system: &dyn SystemId,
) -> BTreeMap<Key, Vec<String>> {
    let index = settings
        .input
        .systems
        .get(system)
        .and_then(|s| s.implicit_keyboard_profile())
        .map(BindingIndex::for_profile)
        .unwrap_or_default();
    conflicting_keys_indexed(settings, topology, system, &index)
}

fn collect_shortcut_conflicts(
    settings: &DesktopSharedSettings,
    mut by_key: BTreeMap<Key, Vec<(&'static str, &'static str)>>,
) -> BTreeMap<Key, Vec<String>> {
    use std::collections::HashMap;

    let mut shortcut_keys = HashMap::with_capacity(settings.input.shortcuts.keyboard.len());
    for binding in &settings.input.shortcuts.keyboard {
        if let Some(key) = binding.key {
            shortcut_keys.entry(binding.action).or_insert(key);
        }
    }
    for descriptor in descriptors::shortcut_descriptors() {
        if let Some(key) = shortcut_keys.get(&descriptor.action) {
            by_key.entry(*key).or_default().push((descriptor.label, ""));
        }
    }

    by_key.retain(|_, labels| labels.len() > 1);
    by_key
        .into_iter()
        .map(|(key, labels)| {
            let strings = labels
                .into_iter()
                .map(|(first, second)| {
                    if second.is_empty() {
                        first.to_string()
                    } else {
                        let mut label = String::with_capacity(first.len() + 1 + second.len());
                        label.push_str(first);
                        label.push(' ');
                        label.push_str(second);
                        label
                    }
                })
                .collect();
            (key, strings)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::model::{
        input::{KeyboardBinding, PersistedControlId, SystemInputSettings},
        shared::DesktopSharedSettings,
    };
    use nerust_core_traits::declare_system_id;
    use nerust_input_traits::{
        AttachmentId, AttachmentSlotDescriptor, ControlDescriptor, DeviceDescriptor, DeviceKindId,
        DigitalControlDescriptor, DigitalControlId, InputTopologyDescriptor, PortDescriptor,
        PortId,
    };
    use nerust_keyboard::Key;

    use super::conflicting_keys;

    declare_system_id!(pub TestSysId, "test");

    const TEST_DEV: DeviceKindId = DeviceKindId::new("test.dev");
    const TEST_ATT: AttachmentId = AttachmentId::new("test.att");

    fn single_port_topology() -> InputTopologyDescriptor {
        InputTopologyDescriptor::new(
            vec![PortDescriptor::new(
                PortId::new("p1"),
                "P1",
                vec![AttachmentSlotDescriptor::new(
                    TEST_ATT,
                    "P1",
                    TEST_DEV,
                    vec![TEST_DEV],
                )],
            )],
            vec![DeviceDescriptor::new(
                TEST_DEV,
                "Test Device",
                vec![
                    ControlDescriptor::Digital(DigitalControlDescriptor::new(
                        DigitalControlId::new("test.ctrl.a"),
                        "A",
                        "A button",
                    )),
                    ControlDescriptor::Digital(DigitalControlDescriptor::new(
                        DigitalControlId::new("test.ctrl.b"),
                        "B",
                        "B button",
                    )),
                ],
            )],
        )
    }

    #[test]
    fn no_conflicts_with_empty_settings() {
        let settings = DesktopSharedSettings::default();
        let conflicts = conflicting_keys(&settings, &single_port_topology(), &TestSysId);
        assert!(conflicts.is_empty());
    }

    #[test]
    fn same_key_different_control_detected() {
        let mut settings = DesktopSharedSettings::default();
        let mut sys = SystemInputSettings::default();
        let profile = sys.implicit_keyboard_profile_mut();
        profile.bindings.push(KeyboardBinding::new(
            TEST_ATT.to_string(),
            PersistedControlId::digital("test.ctrl.a".to_string()),
            Key::KeyZ,
        ));
        profile.bindings.push(KeyboardBinding::new(
            TEST_ATT.to_string(),
            PersistedControlId::digital("test.ctrl.b".to_string()),
            Key::KeyZ,
        ));
        settings.input.systems.insert(Box::new(TestSysId), sys);

        let conflicts = conflicting_keys(&settings, &single_port_topology(), &TestSysId);
        assert!(conflicts.contains_key(&Key::KeyZ));
    }

    #[test]
    fn different_keys_not_conflicting() {
        let mut settings = DesktopSharedSettings::default();
        let mut sys = SystemInputSettings::default();
        let profile = sys.implicit_keyboard_profile_mut();
        profile.bindings.push(KeyboardBinding::new(
            TEST_ATT.to_string(),
            PersistedControlId::digital("test.ctrl.a".to_string()),
            Key::KeyZ,
        ));
        profile.bindings.push(KeyboardBinding::new(
            TEST_ATT.to_string(),
            PersistedControlId::digital("test.ctrl.b".to_string()),
            Key::KeyX,
        ));
        settings.input.systems.insert(Box::new(TestSysId), sys);

        let conflicts = conflicting_keys(&settings, &single_port_topology(), &TestSysId);
        assert!(!conflicts.contains_key(&Key::KeyZ));
        assert!(!conflicts.contains_key(&Key::KeyX));
    }

    #[test]
    fn keyboard_labels_keep_attachment_control_form() {
        let mut settings = DesktopSharedSettings::default();
        let mut sys = SystemInputSettings::default();
        let profile = sys.implicit_keyboard_profile_mut();
        profile.bindings.push(KeyboardBinding::new(
            TEST_ATT.to_string(),
            PersistedControlId::digital("test.ctrl.a".to_string()),
            Key::KeyZ,
        ));
        profile.bindings.push(KeyboardBinding::new(
            TEST_ATT.to_string(),
            PersistedControlId::digital("test.ctrl.b".to_string()),
            Key::KeyZ,
        ));
        settings.input.systems.insert(Box::new(TestSysId), sys);

        let conflicts = conflicting_keys(&settings, &single_port_topology(), &TestSysId);
        let labels = &conflicts[&Key::KeyZ];
        assert_eq!(labels.len(), 2);
        assert!(
            labels.iter().all(|label| label.starts_with("P1 ")),
            "{labels:?}"
        );
    }

    #[test]
    fn shortcut_conflicts_detected_without_profile() {
        use crate::model::input::{ShortcutAction, ShortcutBinding};
        let mut settings = DesktopSharedSettings::default();
        settings.input.shortcuts.keyboard.clear();
        settings.input.shortcuts.keyboard.push(ShortcutBinding {
            action: ShortcutAction::SaveActiveSlot,
            key: Some(Key::F5),
        });
        settings.input.shortcuts.keyboard.push(ShortcutBinding {
            action: ShortcutAction::Reset,
            key: Some(Key::F5),
        });
        let conflicts = conflicting_keys(&settings, &single_port_topology(), &TestSysId);
        let labels = &conflicts[&Key::F5];
        assert!(
            labels.contains(&"Save Active Slot".to_string()),
            "{labels:?}"
        );
        assert!(labels.contains(&"Reset".to_string()), "{labels:?}");
    }

    #[test]
    fn precomputed_index_matches_fresh_scan_and_reuses() {
        use super::{BindingIndex, conflicting_keys_indexed};
        let mut settings = DesktopSharedSettings::default();
        let mut sys = SystemInputSettings::default();
        let profile = sys.implicit_keyboard_profile_mut();
        profile.bindings.push(KeyboardBinding::new(
            TEST_ATT.to_string(),
            PersistedControlId::digital("test.ctrl.a".to_string()),
            Key::KeyZ,
        ));
        profile.bindings.push(KeyboardBinding::new(
            TEST_ATT.to_string(),
            PersistedControlId::digital("test.ctrl.b".to_string()),
            Key::KeyZ,
        ));
        let snapshot = sys.implicit_keyboard_profile().expect("profile").clone();
        settings.input.systems.insert(Box::new(TestSysId), sys);
        let topology = single_port_topology();
        let fresh = conflicting_keys(&settings, &topology, &TestSysId);
        assert!(fresh.contains_key(&Key::KeyZ));
        // One index build, probed across repeated validations.
        let index = BindingIndex::for_profile(&snapshot);
        assert_eq!(index.len(), 2);
        assert!(!index.is_empty());
        assert_eq!(index.key_for("test.att", "test.ctrl.a"), Some(Key::KeyZ));
        assert_eq!(index.key_for("test.att", "missing"), None);
        assert_eq!(
            conflicting_keys_indexed(&settings, &topology, &TestSysId, &index),
            fresh
        );
        assert_eq!(
            conflicting_keys_indexed(&settings, &topology, &TestSysId, &index),
            fresh
        );
    }
}

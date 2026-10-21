//! Settings codec: dialog payload, keyed decode, and apply plan.
//! Pure decisions; the frontend executes the returned plan.

use std::collections::BTreeMap;

use nerust_gui_runtime::settings::SettingsSnapshot;
#[cfg(test)]
use nerust_settings_core::model::local::{
    NotificationAreaMode, NotificationPosition, RumbleTarget, ScreenOrientation,
};
use nerust_settings_core::model::shared::StoragePolicy;
use nerust_system_traits::factory::CoreFactory;

use super::settings_model::{AndroidSettings, SETTINGS_SCHEMA_VERSION};
use super::settings_schema::SettingDef;

impl AndroidSettings {
    pub(super) fn from_keyed_indices(
        values: &BTreeMap<String, usize>,
        current: &Self,
        defs: &[SettingDef],
    ) -> Option<Self> {
        if values
            .keys()
            .any(|key| !defs.iter().any(|def| def.key.as_str() == key.as_str()))
        {
            return None;
        }
        if defs
            .iter()
            .any(|def| def.visible && !values.contains_key(&def.key))
        {
            return None;
        }
        Self::decode_with(current, &|key| values.get(key).copied())
    }

    /// Shared decode core over a key→index lookup for the dialog codec.
    fn decode_with(current: &Self, by_key: &dyn Fn(&str) -> Option<usize>) -> Option<Self> {
        super::settings_schema::decode(current, by_key)
    }

    /// Locked payload builder: `begin_request` already holds the bridge state.
    pub(super) fn dialog_payload_from_defs(request_id: u64, defs: &[SettingDef]) -> String {
        let mut sections: Vec<(String, Vec<serde_json::Value>)> = Vec::new();
        for def in defs {
            let section = if def.key.starts_with("system.") {
                def.key.split('.').take(2).collect::<Vec<_>>().join(".")
            } else if def.key.starts_with("controls.") {
                "controls".to_string()
            } else if def.key.starts_with("storage.") {
                "storage".to_string()
            } else if def.key.starts_with("notifications.") || def.key.starts_with("status.") {
                "notifications".to_string()
            } else if def.key == "vsync" || def.key.starts_with("screen.") {
                "video".to_string()
            } else {
                "audio".to_string()
            };
            let mut field = serde_json::json!({
                    "key": def.key,
                    "label": def.label,
                    "kind": "choice",
                    "value": def.current,
                    "visible": def.visible,
                    "enabled": true,
            });
            if let Some(range) = def.range {
                field["kind"] = serde_json::json!("range");
                field["min"] = serde_json::json!(range.min);
                field["max"] = serde_json::json!(range.max);
                field["unit"] = serde_json::json!(range.unit);
            } else {
                field["options"] = serde_json::json!(def.choices);
            }
            if let Some((_, fields)) = sections.iter_mut().find(|(id, _)| id == &section) {
                fields.push(field);
            } else {
                sections.push((section, vec![field]));
            }
        }
        let mut sections: Vec<_> = sections
            .into_iter()
            .map(|(id, fields)| {
                serde_json::json!({
                    "id": id,
                    "label": crate::settings_sections::section_label(&id),
                    "fields": fields,
                })
            })
            .collect();
        // Canonical order owned by Rust; extras keep discovery order.
        sections.sort_by_key(|section| {
            let id = section["id"].as_str().unwrap_or_default();
            crate::settings_sections::SETTINGS_SECTION_IDS
                .iter()
                .position(|known| *known == id)
                .unwrap_or(usize::MAX)
        });
        serde_json::json!({
            "schemaVersion": SETTINGS_SCHEMA_VERSION,
            "requestId": request_id,
            "sections": sections,
        })
        .to_string()
    }
}

pub(super) enum SettingsPlan {
    /// Decoded snapshot ready to apply (overlay/cache follow-ups included).
    Apply { next: SettingsSnapshot },
    /// Applying needs a document tree first: the frontend stashes `next`
    /// as pending and opens the directory picker.
    NeedDocumentTree { next: SettingsSnapshot },
    /// Undecodable or unapplicable payload: log `reason`, apply nothing.
    Invalid { reason: String },
}

pub(super) struct SettingsApplier;

impl SettingsApplier {
    pub(super) fn plan(
        values: &BTreeMap<String, usize>,
        current: &AndroidSettings,
        defs: &[SettingDef],
        snapshot: SettingsSnapshot,
        factories: &[std::sync::Arc<dyn CoreFactory>],
    ) -> SettingsPlan {
        let Some(android_settings) = AndroidSettings::from_keyed_indices(values, current, defs)
        else {
            return SettingsPlan::Invalid {
                reason: "Android settings dialog returned invalid keyed values".to_string(),
            };
        };
        let previous_storage_policy = snapshot.shared.persistence.storage_policy;
        let mut next = snapshot;
        if let Err(error) = android_settings.apply_to_snapshot(&mut next, factories) {
            return SettingsPlan::Invalid {
                reason: format!("failed to update Android system settings: {error}"),
            };
        }
        if android_settings.storage_policy != StoragePolicy::AppSharedData
            && (android_settings.storage_policy != previous_storage_policy
                || next.shared.persistence.storage_document_tree_uri.is_none())
        {
            return SettingsPlan::NeedDocumentTree { next };
        }
        if android_settings.storage_policy == StoragePolicy::AppSharedData {
            next.shared.persistence.storage_document_tree_uri = None;
        }
        SettingsPlan::Apply { next }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::super::settings_schema::RangeSpec;
    use crate::settings_sections::{SETTINGS_SECTION_IDS, section_label};

    use nerust_gba_factory::GbaFactory;
    use nerust_gba_settings::GbaSettings;
    use nerust_gbc_factory::GbcFactory;
    use nerust_gbc_settings::GbcSettings;
    use nerust_gui_runtime::settings::SettingsSnapshot;
    use nerust_gui_shell::registry::SystemRegistry;
    use nerust_nes_factory::NesFactory;
    use nerust_nes_settings::{NesSettings, NesVideoFilter};
    use nerust_settings_core::model::{
        app_state::DesktopAppState, local::HostBackendLocalSettings, shared::DesktopSharedSettings,
    };
    use nerust_system_traits::factory::CoreFactory;
    use nerust_system_traits::factory::descriptor::SystemSettingsChoiceId;

    use super::*;

    fn default_snapshot() -> SettingsSnapshot {
        let mut shared = DesktopSharedSettings::default();
        shared.systems.insert(
            NesFactory.system_id(),
            Box::new(NesSettings::default()) as Box<dyn nerust_settings_traits::SystemSettings>,
        );
        shared.systems.insert(
            GbcFactory.system_id(),
            Box::new(GbcSettings::default()) as Box<dyn nerust_settings_traits::SystemSettings>,
        );
        shared.systems.insert(
            GbaFactory.system_id(),
            Box::new(GbaSettings::default()) as Box<dyn nerust_settings_traits::SystemSettings>,
        );
        SettingsSnapshot {
            shared,
            local: HostBackendLocalSettings::default(),
            app_state: DesktopAppState::default(),
        }
    }

    fn registry() -> SystemRegistry {
        SystemRegistry::new(vec![
            Arc::new(NesFactory),
            Arc::new(GbcFactory),
            Arc::new(GbaFactory),
        ])
    }

    fn android_settings(snapshot: &SettingsSnapshot, registry: &SystemRegistry) -> AndroidSettings {
        AndroidSettings::from_snapshot(snapshot, registry.all())
    }

    fn set_system_choice(settings: &mut AndroidSettings, field_id: &str, choice_id: &str) {
        let choice = settings
            .system_choices
            .iter_mut()
            .find(|choice| choice.field_id.as_str() == field_id)
            .expect("system field should exist");
        choice.selected = SystemSettingsChoiceId(choice_id.to_string().into());
    }

    /// Current choice indices in dialog order, via the single defs list.
    fn def_currents(android: &AndroidSettings) -> Vec<usize> {
        android
            .setting_defs()
            .into_iter()
            .map(|def| def.current)
            .collect()
    }

    /// Current values keyed by dialog key: the production decode input.
    fn keyed_currents(android: &AndroidSettings) -> BTreeMap<String, usize> {
        android
            .setting_defs()
            .into_iter()
            .map(|def| (def.key, def.current))
            .collect()
    }

    #[test]
    fn cache_reuses_current_defs_and_refreshes_changed_settings() {
        let registry = registry();
        let mut current = android_settings(&default_snapshot(), &registry);
        current.prioritize_system(Some(GbaFactory.system_id().as_ref()));
        assert_eq!(current.system_choices[0].system_id, GbaFactory.system_id());
        let mut cache = None;
        let first = AndroidSettings::cached_setting_defs_locked(&mut cache, &current);
        let second = AndroidSettings::cached_setting_defs_locked(&mut cache, &current);
        assert!(Arc::ptr_eq(&first, &second));
        current.audio_muted = !current.audio_muted;
        let changed = AndroidSettings::cached_setting_defs_locked(&mut cache, &current);
        assert!(!Arc::ptr_eq(&first, &changed));
        assert_eq!(*changed, current.setting_defs());
    }

    #[test]
    fn every_schema_value_round_trips_and_first_invalid_value_is_rejected() {
        let registry = registry();
        let current = android_settings(&default_snapshot(), &registry);
        let defs = current.setting_defs();
        for def in &defs {
            let count = def.range.map_or(def.choices.len(), |range| {
                (range.max - range.min + 1) as usize
            });
            for index in 0..count {
                let mut values = keyed_currents(&current);
                values.insert(def.key.clone(), index);
                let decoded =
                    AndroidSettings::from_keyed_indices(&values, &current, &defs).unwrap();
                assert_eq!(
                    decoded
                        .setting_defs()
                        .iter()
                        .find(|row| row.key == def.key)
                        .unwrap()
                        .current,
                    index,
                    "{}",
                    def.key
                );
            }
            for invalid in [count, usize::MAX] {
                let mut values = keyed_currents(&current);
                values.insert(def.key.clone(), invalid);
                assert!(
                    AndroidSettings::from_keyed_indices(&values, &current, &defs).is_none(),
                    "{}={invalid}",
                    def.key
                );
            }
        }
    }

    #[test]
    fn round_trips_default_snapshot() {
        let snapshot = default_snapshot();
        let registry = registry();
        let android = android_settings(&snapshot, &registry);
        let mut out = default_snapshot();
        android.apply_to_snapshot(&mut out, registry.all()).unwrap();
        // The round-trip should not change anything when starting from defaults.
        assert_eq!(out.local.audio.muted, snapshot.local.audio.muted);
        assert_eq!(
            out.local.audio.master_volume_percent,
            snapshot.local.audio.master_volume_percent
        );
        assert_eq!(out.local.audio.latency_ms, snapshot.local.audio.latency_ms);
        assert_eq!(
            out.local.audio.sample_rate,
            snapshot.local.audio.sample_rate
        );
        assert_eq!(
            out.local.video.presentation.vsync,
            snapshot.local.video.presentation.vsync
        );
    }

    #[test]
    fn from_snapshot_extracts_nes_filter() {
        let mut snapshot = default_snapshot();
        let nes = snapshot
            .shared
            .systems
            .get_mut(NesFactory.system_id().as_ref())
            .map(|s| s.downcast_mut::<NesSettings>().unwrap())
            .unwrap();
        nes.video.filter = NesVideoFilter::NtscSVideo;

        let registry = registry();
        let android = android_settings(&snapshot, &registry);
        let filter = android
            .system_choices
            .iter()
            .find(|choice| choice.field_id.as_str() == "video.filter")
            .unwrap();
        assert_eq!(filter.selected.as_str(), "ntsc_svideo");
    }

    #[test]
    fn apply_to_snapshot_writes_all_fields() {
        let registry = registry();
        let mut snapshot = default_snapshot();
        let mut android = android_settings(&snapshot, &registry);
        android.audio_muted = true;
        android.master_volume_percent = 50;
        android.latency_ms = 75;
        android.sample_rate = 44_100;
        android.vsync = false;
        android.motion_enabled = false;
        android.rumble_enabled = false;
        android.rumble_strength_percent = 35;
        android.rumble_target = RumbleTarget::Controller;
        android.notifications_info_enabled = false;
        android.notifications_position = NotificationPosition::TopRight;
        android.notifications_area_mode = NotificationAreaMode::Strip;
        android.status_enabled = true;
        android.status_position = NotificationPosition::BottomRight;
        android.status_area_mode = NotificationAreaMode::Strip;
        set_system_choice(&mut android, "video.filter", "ntsc_rgb");
        android
            .apply_to_snapshot(&mut snapshot, registry.all())
            .unwrap();

        assert!(snapshot.local.audio.muted);
        assert_eq!(snapshot.local.audio.master_volume_percent, 50);
        assert_eq!(snapshot.local.audio.latency_ms, 75);
        assert_eq!(snapshot.local.audio.sample_rate, 44_100);
        assert!(!snapshot.local.video.presentation.vsync);
        assert!(!snapshot.local.cartridge_peripherals.motion_enabled);
        assert!(!snapshot.local.cartridge_peripherals.rumble_enabled);
        assert_eq!(
            snapshot.local.cartridge_peripherals.rumble_strength_percent,
            35
        );
        assert_eq!(
            snapshot.local.cartridge_peripherals.rumble_target,
            RumbleTarget::Controller
        );
        assert!(!snapshot.local.notifications.info_enabled);
        assert_eq!(
            snapshot.local.notifications.position,
            NotificationPosition::TopRight
        );
        assert_eq!(
            snapshot.local.notifications.area_mode,
            NotificationAreaMode::Strip
        );
        assert!(snapshot.local.notifications.status_enabled);
        assert_eq!(
            snapshot.local.notifications.status_position,
            NotificationPosition::BottomRight
        );
        assert_eq!(
            snapshot.local.notifications.status_area_mode,
            NotificationAreaMode::Strip
        );
        let nes = snapshot
            .shared
            .systems
            .get(NesFactory.system_id().as_ref())
            .map(|s| s.downcast_ref::<NesSettings>().unwrap())
            .unwrap();
        assert_eq!(nes.video.filter, NesVideoFilter::NtscRgb);
    }

    #[test]
    fn current_indices_matches_defaults() {
        let snapshot = default_snapshot();
        let registry = registry();
        let android = android_settings(&snapshot, &registry);
        let indices = def_currents(&android);
        // Default indices: unmuted 0, volume 100, latency 40, rate 1, vsync 1.
        // Notifications on, BottomLeft, Overlay; status off, TopLeft.
        assert_eq!(
            &indices[..24],
            [
                0, 100, 40, 1, 1, 0, 0, 1, 65, 50, 30, 1, 1, 1, 100, 0, 1, 1, 1, 0, 0, 0, 2, 0
            ]
        );
        assert_eq!(indices.len(), 29);
    }

    #[test]
    fn dialog_payload_contains_gba_system_key() {
        let registry = registry();
        let android = android_settings(&default_snapshot(), &registry);
        let keys: Vec<String> = android
            .setting_defs()
            .into_iter()
            .map(|def| def.key)
            .collect();
        assert!(keys.iter().any(|key| key.starts_with("system.gba.")));
    }

    #[test]
    fn from_keyed_indices_round_trips() {
        let registry = registry();
        let mut snapshot = default_snapshot();
        let mut original = android_settings(&snapshot, &registry);
        original.audio_muted = true;
        original.master_volume_percent = 25;
        original.latency_ms = 100;
        original.sample_rate = 44_100;
        original.vsync = false;
        original.screen_orientation = ScreenOrientation::Landscape;
        original.motion_enabled = false;
        original.rumble_enabled = false;
        original.rumble_strength_percent = 42;
        original.rumble_target = RumbleTarget::Handset;
        set_system_choice(&mut original, "video.filter", "ntsc_svideo");
        original
            .apply_to_snapshot(&mut snapshot, registry.all())
            .unwrap();
        let recovered = android_settings(&snapshot, &registry);
        let parsed = AndroidSettings::from_keyed_indices(
            &keyed_currents(&recovered),
            &recovered,
            &recovered.setting_defs(),
        )
        .unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn from_keyed_indices_round_trips_non_default_audio_values() {
        let registry = registry();
        let mut original = android_settings(&default_snapshot(), &registry);
        original.audio_muted = false;
        original.master_volume_percent = 83;
        original.latency_ms = 37;
        original.sample_rate = 44_100;
        original.vsync = true;
        original.screen_orientation = ScreenOrientation::Portrait;
        set_system_choice(&mut original, "video.filter", "none");

        let parsed = AndroidSettings::from_keyed_indices(
            &keyed_currents(&original),
            &original,
            &original.setting_defs(),
        )
        .unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn from_keyed_indices_rejects_out_of_range() {
        let registry = registry();
        let current = android_settings(&default_snapshot(), &registry);
        let mut bad = keyed_currents(&current);
        bad.insert("master_volume".to_string(), 500);
        assert!(
            AndroidSettings::from_keyed_indices(&bad, &current, &current.setting_defs()).is_none()
        );
        let mut bad = keyed_currents(&current);
        bad.insert("audio_muted".to_string(), 7);
        assert!(
            AndroidSettings::from_keyed_indices(&bad, &current, &current.setting_defs()).is_none()
        );
    }

    #[test]
    fn dialog_choices_cover_full_android_audio_range() {
        let registry = registry();
        let android = android_settings(&default_snapshot(), &registry);
        let defs = android.setting_defs();
        let by_key = |key: &str| defs.iter().find(|def| def.key == key).unwrap();
        let volume = by_key("master_volume").range.unwrap();
        assert_eq!((volume.min, volume.max, volume.unit), (0, 100, "%"));
        let latency = by_key("latency_ms").range.unwrap();
        assert_eq!((latency.min, latency.max, latency.unit), (10, 200, " ms"));
        let sample_rate = by_key("sample_rate");
        assert!(sample_rate.range.is_none());
        assert!(!sample_rate.choices.is_empty());
        for choice in &sample_rate.choices {
            let Some(rate_str) = choice.strip_suffix(" Hz") else {
                panic!("sample rate choice '{choice}' must end with ' Hz'");
            };
            let rate: u32 = rate_str
                .parse()
                .expect("sample rate must be a valid integer");
            assert!(
                (1..=192_000).contains(&rate),
                "sample rate {rate} must be within 1..=192000"
            );
        }
    }

    #[test]
    fn setting_defs_are_internally_consistent() {
        // Every row must point at a valid choice, carry a unique key,
        // and either list choices or declare a range.
        let snapshot = default_snapshot();
        let registry = registry();
        let android = android_settings(&snapshot, &registry);
        let defs = android.setting_defs();
        let mut keys = std::collections::BTreeSet::new();
        for def in &defs {
            assert!(keys.insert(def.key.clone()), "duplicate key {}", def.key);
            assert!(!def.label.is_empty(), "empty label for {}", def.key);
            match def.range {
                Some(range) => {
                    assert!(range.min <= range.max, "empty range for {}", def.key);
                    assert!(def.choices.is_empty(), "range rows list no choices");
                    let value = range.min + def.current as i64;
                    assert!((range.min..=range.max).contains(&value));
                }
                None => {
                    assert!(!def.choices.is_empty(), "no choices for {}", def.key);
                    assert!(
                        def.current < def.choices.len(),
                        "current {} out of range for {}",
                        def.current,
                        def.key
                    );
                }
            }
        }
    }

    #[test]
    fn dialog_choices_align_with_keys() {
        // Regression test: each key must carry its own setting's choice list.
        // A past misalignment silently rejected valid saves.
        let registry = registry();
        let android = android_settings(&default_snapshot(), &registry);
        let defs = android.setting_defs();
        let at = |key: &str| {
            defs.iter()
                .find(|def| def.key == key)
                .unwrap()
                .choices
                .join("\t")
        };
        let range_at = |key: &str| {
            defs.iter()
                .find(|def| def.key == key)
                .unwrap()
                .range
                .unwrap()
        };
        assert_eq!(at("screen.orientation"), "Auto Rotate\tPortrait\tLandscape");
        assert_eq!(
            at("storage.policy"),
            "Next to ROM\tApp Storage\tCustom Directory"
        );
        assert_eq!(at("controls.overlay.visibility"), "Always\tAuto\tHidden");
        assert_eq!(
            range_at("controls.overlay.opacity"),
            RangeSpec {
                min: 0,
                max: 100,
                unit: "%"
            }
        );
        assert_eq!(
            range_at("controls.overlay.scale"),
            RangeSpec {
                min: 50,
                max: 150,
                unit: "%"
            }
        );
        assert_eq!(
            range_at("controls.overlay.vertical_offset"),
            RangeSpec {
                min: -30,
                max: 30,
                unit: "%"
            }
        );
        assert_eq!(at("controls.overlay.haptics"), "Off\tOn");
        assert_eq!(at("controls.cartridge.motion"), "Off\tOn");
        assert_eq!(at("controls.cartridge.rumble"), "Off\tOn");
        assert_eq!(
            range_at("controls.cartridge.rumble_strength"),
            RangeSpec {
                min: 0,
                max: 100,
                unit: "%"
            }
        );
        assert_eq!(
            at("controls.cartridge.rumble_target"),
            "Auto\tHandset\tController"
        );
    }

    #[test]
    fn dialog_payload_contains_schema_request_and_stable_keys() {
        let registry = registry();
        let android = android_settings(&default_snapshot(), &registry);
        let defs = android.setting_defs();
        let payload: serde_json::Value =
            serde_json::from_str(&AndroidSettings::dialog_payload_from_defs(42, &defs)).unwrap();

        assert_eq!(payload["schemaVersion"], SETTINGS_SCHEMA_VERSION);
        assert_eq!(payload["requestId"], 42);
        assert_eq!(payload["sections"][0]["id"], "audio");
        assert_eq!(payload["sections"][0]["fields"][0]["key"], "audio_muted");
        let fields: Vec<_> = payload["sections"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|section| section["fields"].as_array().unwrap())
            .collect();
        let keys: Vec<_> = fields
            .iter()
            .map(|field| field["key"].as_str().unwrap())
            .collect();
        let volume = fields
            .iter()
            .find(|field| field["key"] == "master_volume")
            .unwrap();
        assert!(volume.get("options").is_none());
        assert_eq!(volume["min"], 0);
        assert_eq!(volume["max"], 100);
        assert_eq!(volume["unit"], "%");
        assert_eq!(volume["visible"], true);
        let storage = fields
            .iter()
            .find(|field| field["key"] == "storage.policy")
            .unwrap();
        assert_eq!(storage["options"][0], "Next to ROM");
        assert_eq!(storage["options"][1], "App Storage");
        let orientation = fields
            .iter()
            .find(|field| field["key"] == "screen.orientation")
            .unwrap();
        assert_eq!(orientation["options"][0], "Auto Rotate");
        assert_eq!(orientation["options"][1], "Portrait");
        assert_eq!(orientation["options"][2], "Landscape");
        assert!(
            payload["sections"]
                .as_array()
                .unwrap()
                .iter()
                .any(|section| {
                    section["id"] == "video"
                        && section["fields"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|field| field["key"] == "screen.orientation")
                })
        );
        assert!(keys.iter().any(|key| key.starts_with("system.gbc.")));
    }

    #[test]
    fn dialog_payload_marks_hidden_area_mode() {
        // Same-edge rule resolved in Rust: `status.area_mode` hides when both
        // positions share an edge; Kotlin only filters on `visible`.
        let registry = registry();
        let android = android_settings(&default_snapshot(), &registry);
        let defs = android.setting_defs();
        let doc: serde_json::Value =
            serde_json::from_str(&AndroidSettings::dialog_payload_from_defs(1, &defs)).unwrap();
        let fields: Vec<&serde_json::Value> = doc["sections"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|section| section["fields"].as_array().unwrap().iter())
            .collect();
        let visible = |key: &str| {
            fields.iter().find(|field| field["key"] == key).unwrap()["visible"]
                .as_bool()
                .unwrap()
        };
        // Defaults: BottomLeft vs TopLeft differ, so the row shows.
        assert!(visible("status.area_mode"));
        assert!(visible("notifications.position"));
        let mut same_edge = android;
        same_edge.status_position = NotificationPosition::BottomRight;
        let same_defs = same_edge.setting_defs();
        let doc: serde_json::Value =
            serde_json::from_str(&AndroidSettings::dialog_payload_from_defs(2, &same_defs))
                .unwrap();
        let area = doc["sections"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|section| section["fields"].as_array().unwrap().iter())
            .find(|field| field["key"] == "status.area_mode")
            .unwrap();
        assert_eq!(area["visible"], false);
        assert!(area.get("edges").is_none());
    }

    #[test]
    fn keyed_indices_do_not_depend_on_map_order() {
        let registry = registry();
        let current = android_settings(&default_snapshot(), &registry);
        let mut values = BTreeMap::new();
        for def in current.setting_defs().into_iter().rev() {
            values.insert(def.key, def.current);
        }

        assert_eq!(
            AndroidSettings::from_keyed_indices(&values, &current, &current.setting_defs()),
            Some(current)
        );
    }

    #[test]
    fn same_edge_preserves_status_area() {
        let registry = registry();
        let mut android = android_settings(&default_snapshot(), &registry);
        android.status_position = NotificationPosition::BottomRight;
        android.status_area_mode = NotificationAreaMode::Strip;
        // Same edge as the default BottomLeft notification: the row hides.
        // The value follows the notification mode at render time.
        let defs = android.setting_defs();
        assert_eq!(defs.len(), 29);
        let area = defs
            .iter()
            .find(|def| def.key == "status.area_mode")
            .unwrap();
        assert!(!area.visible);
        let back = AndroidSettings::from_keyed_indices(
            &keyed_currents(&android),
            &android,
            &android.setting_defs(),
        )
        .unwrap();
        assert_eq!(back.status_position, NotificationPosition::BottomRight);
        assert_eq!(back.status_area_mode, NotificationAreaMode::Strip);
    }

    #[test]
    fn dialog_payload_sections_carry_rust_owned_labels() {
        let registry = registry();
        let android = android_settings(&default_snapshot(), &registry);
        let defs = android.setting_defs();
        let doc: serde_json::Value =
            serde_json::from_str(&AndroidSettings::dialog_payload_from_defs(7, &defs)).unwrap();
        let sections = doc["sections"].as_array().unwrap();
        let ids: Vec<&str> = sections
            .iter()
            .map(|section| section["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, SETTINGS_SECTION_IDS);
        for section in sections {
            let id = section["id"].as_str().unwrap();
            assert_eq!(section["label"].as_str().unwrap(), section_label(id));
        }
        let fields: Vec<&serde_json::Value> = sections
            .iter()
            .flat_map(|section| section["fields"].as_array().unwrap().iter())
            .collect();
        let volume = fields
            .iter()
            .find(|field| field["key"] == "master_volume")
            .unwrap();
        assert_eq!(volume["kind"], "range");
        assert_eq!(volume["min"], 0);
        assert_eq!(volume["max"], 100);
        assert_eq!(volume["unit"], "%");
        let muted = fields
            .iter()
            .find(|field| field["key"] == "audio_muted")
            .unwrap();
        assert_eq!(muted["kind"], "choice");
        assert!(muted.get("min").is_none());
    }

    #[test]
    fn keyed_indices_tolerate_absent_invisible_rows() {
        // Same-edge hides `status.area_mode`; an omitted invisible row
        // keeps the current value instead of rejecting the payload.
        let registry = registry();
        let mut android = android_settings(&default_snapshot(), &registry);
        android.status_position = NotificationPosition::BottomRight;
        android.status_area_mode = NotificationAreaMode::Strip;
        let mut values = current_values(&android);
        assert!(
            !android
                .setting_defs()
                .iter()
                .find(|def| def.key == "status.area_mode")
                .unwrap()
                .visible
        );
        values.remove("status.area_mode");
        let defs = android.setting_defs();
        let back = AndroidSettings::from_keyed_indices(&values, &android, &defs).unwrap();
        assert_eq!(back.status_area_mode, NotificationAreaMode::Strip);
        values.insert("status.area_mode".to_string(), 0);
        let back = AndroidSettings::from_keyed_indices(&values, &android, &defs).unwrap();
        assert_eq!(back.status_area_mode, NotificationAreaMode::Overlay);
    }

    #[test]
    fn keyed_indices_reject_unknown_keys_and_missing_visible_rows() {
        let registry = registry();
        let android = android_settings(&default_snapshot(), &registry);
        let mut values = current_values(&android);
        values.insert("mystery.key".to_string(), 0);
        assert!(
            AndroidSettings::from_keyed_indices(&values, &android, &android.setting_defs())
                .is_none()
        );
        let mut values = current_values(&android);
        values.remove("audio_muted");
        assert!(
            AndroidSettings::from_keyed_indices(&values, &android, &android.setting_defs())
                .is_none()
        );
    }

    #[test]
    fn defs_cache_tracks_model_changes_without_a_parallel_field_list() {
        let registry = registry();
        let base = android_settings(&default_snapshot(), &registry);
        let mut cache = None;
        let first = AndroidSettings::cached_setting_defs_locked(&mut cache, &base);
        let mut flip = base.clone();
        flip.audio_muted = !flip.audio_muted;
        assert!(!Arc::ptr_eq(
            &first,
            &AndroidSettings::cached_setting_defs_locked(&mut cache, &flip)
        ));
        let mut flip = base.clone();
        flip.master_volume_percent = flip.master_volume_percent.wrapping_add(1);
        assert_eq!(
            *AndroidSettings::cached_setting_defs_locked(&mut cache, &flip),
            flip.setting_defs()
        );
        let mut flip = base.clone();
        flip.latency_ms = flip.latency_ms.wrapping_add(1);
        assert_eq!(
            *AndroidSettings::cached_setting_defs_locked(&mut cache, &flip),
            flip.setting_defs()
        );
        let mut flip = base.clone();
        flip.screen_orientation = ScreenOrientation::Landscape;
        assert_eq!(
            *AndroidSettings::cached_setting_defs_locked(&mut cache, &flip),
            flip.setting_defs()
        );
        let mut flip = base.clone();
        flip.overlay_vertical_offset_percent = flip.overlay_vertical_offset_percent.wrapping_add(1);
        assert_eq!(
            *AndroidSettings::cached_setting_defs_locked(&mut cache, &flip),
            flip.setting_defs()
        );
        let mut flip = base.clone();
        flip.notifications_position = NotificationPosition::TopRight;
        assert_eq!(
            *AndroidSettings::cached_setting_defs_locked(&mut cache, &flip),
            flip.setting_defs()
        );
        let mut flip = base.clone();
        flip.status_area_mode = NotificationAreaMode::Strip;
        assert_eq!(
            *AndroidSettings::cached_setting_defs_locked(&mut cache, &flip),
            flip.setting_defs()
        );
        let mut flip = base.clone();
        flip.rumble_target = RumbleTarget::Controller;
        assert_eq!(
            *AndroidSettings::cached_setting_defs_locked(&mut cache, &flip),
            flip.setting_defs()
        );
    }

    fn current_values(android: &AndroidSettings) -> BTreeMap<String, usize> {
        android
            .setting_defs()
            .into_iter()
            .map(|def| (def.key, def.current))
            .collect()
    }

    #[test]
    fn settings_plan_applies_current_values() {
        let registry = registry();
        let mut snapshot = default_snapshot();
        // App storage needs no document tree: the unchanged round-trip
        // applies directly.
        snapshot.shared.persistence.storage_policy = StoragePolicy::AppSharedData;
        let current = android_settings(&snapshot, &registry);
        match SettingsApplier::plan(
            &current_values(&current),
            &current,
            &current.setting_defs(),
            snapshot.clone(),
            registry.all(),
        ) {
            SettingsPlan::Apply { next } => {
                assert_eq!(
                    next.shared.persistence.storage_policy,
                    current.storage_policy
                );
                assert!(
                    next.shared.persistence.storage_document_tree_uri.is_none(),
                    "app storage clears the tree URI"
                );
            }
            SettingsPlan::NeedDocumentTree { .. } => {
                panic!("current values must not need a document tree")
            }
            SettingsPlan::Invalid { reason } => panic!("current values must apply: {reason}"),
        }
    }

    #[test]
    fn settings_plan_routes_custom_directory_to_tree_picker() {
        let registry = registry();
        let snapshot = default_snapshot();
        let current = android_settings(&snapshot, &registry);
        let mut android = current.clone();
        android.storage_policy = StoragePolicy::CustomDirectory;
        match SettingsApplier::plan(
            &current_values(&android),
            &current,
            &current.setting_defs(),
            snapshot.clone(),
            registry.all(),
        ) {
            SettingsPlan::NeedDocumentTree { next } => {
                assert_eq!(
                    next.shared.persistence.storage_policy,
                    StoragePolicy::CustomDirectory
                );
            }
            SettingsPlan::Apply { .. } => {
                panic!("custom directory without a tree URI must need the picker")
            }
            SettingsPlan::Invalid { reason } => panic!("custom directory must plan: {reason}"),
        }
    }

    #[test]
    fn settings_plan_rejects_garbage_values() {
        let registry = registry();
        let snapshot = default_snapshot();
        let current = android_settings(&snapshot, &registry);
        match SettingsApplier::plan(
            &BTreeMap::new(),
            &current,
            &current.setting_defs(),
            snapshot.clone(),
            registry.all(),
        ) {
            SettingsPlan::Invalid { .. } => {}
            _ => panic!("empty values must be invalid"),
        }
    }
}

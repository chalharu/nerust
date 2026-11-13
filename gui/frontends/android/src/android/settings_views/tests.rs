//! Settings wire tests: payload shape, capture round-trips, review allowlists.
use super::{capture::*, commands::*, open_payload, projection::*, wire::*};
use std::sync::Arc;

use crate::bridge_contract::SETTINGS_SCHEMA_VERSION;
use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_settings_core::{
    i18n::{I18n, UiText},
    model::language::AppLanguage,
};
use nerust_system_traits::factory::CoreFactory;

use nerust_gba_factory::GbaFactory;
use nerust_gba_settings::GbaSettings;
use nerust_gbc_factory::GbcFactory;
use nerust_gbc_settings::GbcSettings;
use nerust_gui_shell::registry::SystemRegistry;
use nerust_nes_factory::NesFactory;
use nerust_nes_settings::{NesSettings, NesVideoFilter};
use nerust_settings_core::model::{
    app_state::DesktopAppState, local::HostBackendLocalSettings, shared::DesktopSharedSettings,
};
use nerust_settings_traits::SystemSettings;

fn default_snapshot() -> SettingsSnapshot {
    let mut shared = DesktopSharedSettings::default();
    shared.systems.insert(
        NesFactory.system_id(),
        Box::new(NesSettings::default()) as Box<dyn SystemSettings>,
    );
    shared.systems.insert(
        GbcFactory.system_id(),
        Box::new(GbcSettings::default()) as Box<dyn SystemSettings>,
    );
    shared.systems.insert(
        GbaFactory.system_id(),
        Box::new(GbaSettings::default()) as Box<dyn SystemSettings>,
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

fn test_deps() -> VmDeps {
    let registry = registry();
    VmDeps {
        factories: registry.all().to_vec(),
        rates: Arc::from([44_100u32, 48_000].as_slice()),
    }
}

fn with_test_deps<T>(operation: impl FnOnce() -> T) -> T {
    // Never reset: tests share the process-wide slot and run in parallel.
    // Installs are equivalent, so first-wins is benign.
    install_vm_deps(test_deps().factories, test_deps().rates);
    operation()
}

fn test_keyboards() -> Vec<KeyboardInfo> {
    vec![
        KeyboardInfo {
            member: "kbd:builtin".to_string(),
            name: "Built-in Keyboard".to_string(),
            builtin: true,
        },
        KeyboardInfo {
            member: "kbd:usb-1".to_string(),
            name: "USB Keyboard".to_string(),
            builtin: false,
        },
    ]
}

fn test_payload() -> (SettingsSnapshot, ViewPayload) {
    test_payload_with_keyboards(&test_keyboards())
}

fn test_payload_with_keyboards(keyboards: &[KeyboardInfo]) -> (SettingsSnapshot, ViewPayload) {
    with_test_deps(|| {
        let snapshot = default_snapshot();
        let payload: ViewPayload =
            open_payload(&snapshot, keyboards, None, &[], None).expect("open payload builds");
        assert_eq!(payload.schema_version, SETTINGS_SCHEMA_VERSION);
        (snapshot, payload)
    })
}

fn row<'a>(payload: &'a ViewPayload, key: &str) -> &'a RowWire {
    payload
        .sections
        .iter()
        .flat_map(|section| &section.rows)
        .find(|row| row.key == key)
        .unwrap_or_else(|| panic!("row {key} exists"))
}

/// Keys of interactive rows without supporting text, for the policy test.
fn payload_bare_keys(payload: &ViewPayload) -> Vec<String> {
    payload
        .sections
        .iter()
        .flat_map(|section| &section.rows)
        .filter(|row| {
            matches!(
                row.kind,
                RowKind::Choice
                    | RowKind::Toggle
                    | RowKind::Number
                    | RowKind::Group
                    | RowKind::Button
            ) && row.description.is_empty()
        })
        .map(|row| row.key.clone())
        .collect()
}

fn apply(
    snapshot: &mut SettingsSnapshot,
    capture: &mut CaptureSlot,
    keyboards: &mut Vec<KeyboardInfo>,
    key: &str,
    op: WireOp,
    index: usize,
    number: i64,
) -> ViewPayload {
    with_test_deps(|| {
        let command_json = serde_json::to_string(&WireCommand {
            key: key.to_string(),
            index,
            number,
            op,
            payload: serde_json::Value::Null,
        })
        .unwrap();
        apply_command_to(
            snapshot,
            &command_json,
            capture,
            keyboards,
            &mut None,
            &mut Vec::new(),
            &mut None,
        )
        .unwrap_or_else(|error| panic!("command failed: {error}"))
    })
}

fn set(snapshot: &mut SettingsSnapshot, key: &str, index: usize, number: i64) -> ViewPayload {
    apply(
        snapshot,
        &mut None,
        &mut test_keyboards(),
        key,
        WireOp::Set,
        index,
        number,
    )
}

#[test]
fn overlay_always_roundtrip_through_finish() {
    use nerust_settings_core::model::local::TouchOverlayVisibility;
    // Auto -> Always must survive the validation gate: the choice write,
    // the save, and the runtime push all read the same snapshot field.
    let (mut snapshot, _) = test_payload();
    let next = set(&mut snapshot, "controls.overlay_visibility", 0, 0);
    assert_eq!(row(&next, "controls.overlay_visibility").selected, 0);
    match finish_snapshot(&snapshot, None) {
        Ok(done) => assert_eq!(
            done.local.touch_overlay.visibility,
            TouchOverlayVisibility::Always
        ),
        Err(issues) => panic!("finish failed: {issues:?}"),
    }
}

#[test]
fn open_payload_covers_core_items() {
    let (_snapshot, payload) = test_payload();
    // Every core row the screen exposes must exist.
    for key in [
        "audio.muted",
        "audio.volume",
        "audio.latency",
        "audio.sample_rate",
        "video.vsync",
        "video.orientation",
        "general.storage_policy",
        "controls.overlay_visibility",
        "controls.overlay_opacity",
        "controls.overlay_scale",
        "controls.overlay_vertical_offset",
        "controls.overlay_haptics",
        "controls.cartridge_motion",
        "controls.cartridge_rumble",
        "controls.cartridge_rumble_strength",
        "controls.cartridge_rumble_target",
        "notifications.info_enabled",
        "notifications.warning_enabled",
        "notifications.error_enabled",
        "notifications.position",
        "notifications.area_mode",
        "status.enabled",
        "status.position",
        "status.area_mode",
    ] {
        row(&payload, key);
    }
    assert!(
        payload
            .sections
            .iter()
            .flat_map(|section| &section.rows)
            .any(|row| row.key.starts_with("system.") && row.key.ends_with("video.filter")),
        "system filter rows exist"
    );
}

#[test]
fn system_locale_resolves_system_default_for_display() {
    // Android never sets LANG, so the vault-pushed tag decides: the
    // same SystemDefault draft projects Japanese with `ja`, English
    // otherwise, and English before the first push.
    for system in [
        Some(AppLanguage::Japanese),
        Some(AppLanguage::English),
        None,
    ] {
        let snapshot = default_snapshot();
        let label = with_test_deps(|| {
            let json = open_payload(&snapshot, &test_keyboards(), None, &[], system)
                .expect("open payload builds");
            let payload: ViewPayload = json;
            payload
                .sections
                .iter()
                .find(|section| section.id == "play")
                .expect("play section exists")
                .label
                .clone()
        });
        let want = if system == Some(AppLanguage::Japanese) {
            "プレイ"
        } else {
            "Play"
        };
        assert_eq!(label, want, "locale {system:?} resolves the display");
    }
    assert_eq!(
        resolve_display(AppLanguage::SystemDefault, Some(AppLanguage::Japanese)).language(),
        AppLanguage::Japanese
    );
    assert_eq!(
        resolve_display(AppLanguage::SystemDefault, Some(AppLanguage::English)).language(),
        AppLanguage::English
    );
    assert_eq!(
        resolve_display(AppLanguage::English, Some(AppLanguage::Japanese)).language(),
        AppLanguage::English
    );
    assert_eq!(
        resolve_display(AppLanguage::English, None).language(),
        AppLanguage::English
    );
}

#[test]
fn language_row_selects_the_draft_choice_not_the_display_copy() {
    // SystemDefault under a `ja` system projects Japanese labels, but
    // the language row must highlight SystemDefault: the VM works on
    // the concretized copy, which would otherwise pin Japanese.
    let snapshot = default_snapshot();
    assert_eq!(
        snapshot.shared.general.language,
        AppLanguage::SystemDefault,
        "default draft leaves the choice to the system"
    );
    let payload: ViewPayload = with_test_deps(|| {
        open_payload(
            &snapshot,
            &test_keyboards(),
            None,
            &[],
            Some(AppLanguage::Japanese),
        )
        .expect("open payload builds")
    });
    let row = row(&payload, "general.language");
    let selected = row.choices[row.selected].value.clone();
    assert_eq!(
        selected,
        serde_json::json!("system_default"),
        "row highlights the draft choice, labels stay resolved: {selected}"
    );
    assert_eq!(row.label, "UI 言語", "labels still resolve to Japanese");
}

#[test]
fn explicit_japanese_under_ja_system_persists_as_japanese() {
    // SystemDefault under a `ja` system resolves to Japanese, so an
    // explicit Japanese choice reads back identical to "untouched".
    // The old pin-back compared values and discarded it as no-change.
    let mut snapshot = default_snapshot();
    let mut keyboards = test_keyboards();
    let mut locale = Some(AppLanguage::Japanese);
    let payload: ViewPayload = with_test_deps(|| {
        open_payload(&snapshot, &keyboards, None, &[], locale).expect("open payload builds")
    });
    let language = row(&payload, "general.language");
    let japanese = language
        .choices
        .iter()
        .position(|choice| choice.value == serde_json::json!("japanese"))
        .expect("japanese choice exists");
    let next: ViewPayload = with_test_deps(|| {
        apply_command_to(
            &mut snapshot,
            &serde_json::to_string(&WireCommand {
                key: "general.language".to_string(),
                index: japanese,
                number: 0,
                op: WireOp::Set,
                payload: serde_json::Value::Null,
            })
            .unwrap(),
            &mut None,
            &mut keyboards,
            &mut None,
            &mut Vec::new(),
            &mut locale,
        )
        .expect("language set applies")
    });
    assert_eq!(
        snapshot.shared.general.language,
        AppLanguage::Japanese,
        "explicit choice survives even when it equals the resolved locale"
    );
    let selected_row = row(&next, "general.language");
    let selected = selected_row.choices[selected_row.selected].value.clone();
    assert_eq!(
        selected,
        serde_json::json!("japanese"),
        "row highlights the explicit choice: {selected}"
    );
}

#[test]
fn display_resolution_never_concretes_the_draft() {
    // Projecting and editing under a `ja` system keeps SystemDefault in
    // the draft (saving must not rewrite the user's choice); an
    // explicit change still persists as-is.
    let (mut snapshot, _) = test_payload();
    let mut keyboards = test_keyboards();
    let mut locale = Some(AppLanguage::Japanese);
    let out = with_test_deps(|| {
        apply_command_to(
            &mut snapshot,
            &serde_json::to_string(&WireCommand {
                key: "audio.volume".to_string(),
                index: 0,
                number: 42,
                op: WireOp::Set,
                payload: serde_json::Value::Null,
            })
            .unwrap(),
            &mut None,
            &mut keyboards,
            &mut None,
            &mut Vec::new(),
            &mut locale,
        )
        .expect("volume set applies")
    });
    let payload: ViewPayload = out;
    assert_eq!(
        payload
            .sections
            .iter()
            .find(|section| section.id == "play")
            .expect("play exists")
            .label,
        "プレイ",
        "projection speaks Japanese"
    );
    assert_eq!(
        snapshot.shared.general.language,
        AppLanguage::SystemDefault,
        "draft keeps the choice"
    );
    // Explicit English under a `ja` system persists as English.
    let english = with_test_deps(|| {
        let json = open_payload(
            &snapshot,
            &test_keyboards(),
            None,
            &[],
            Some(AppLanguage::Japanese),
        )
        .expect("open payload builds");
        let probe: ViewPayload = json;
        probe
            .sections
            .iter()
            .flat_map(|section| &section.rows)
            .find(|row| row.key == "general.language")
            .expect("language row exists")
            .choices
            .iter()
            .position(|choice| choice.value == serde_json::json!("english"))
            .expect("english choice exists")
    });
    let out = with_test_deps(|| {
        apply_command_to(
            &mut snapshot,
            &serde_json::to_string(&WireCommand {
                key: "general.language".to_string(),
                index: english,
                number: 0,
                op: WireOp::Set,
                payload: serde_json::Value::Null,
            })
            .unwrap(),
            &mut None,
            &mut keyboards,
            &mut None,
            &mut Vec::new(),
            &mut locale,
        )
        .expect("language set applies")
    });
    let _payload: ViewPayload = out;
    assert_eq!(snapshot.shared.general.language, AppLanguage::English);
}

#[test]
fn locale_op_upserts_without_mutating_draft() {
    let (mut snapshot, _) = test_payload();
    let before = snapshot.clone();
    let mut locale: Option<AppLanguage> = None;
    let command_json = serde_json::to_string(&WireCommand {
        key: "device_truth".to_string(),
        index: 0,
        number: 0,
        op: WireOp::DeviceTruth,
        payload: serde_json::json!({
            "keyboards": serde_json::to_value(test_keyboards()).unwrap(),
            "capable": true,
            "gamepads": [],
            "lang": "ja",
        }),
    })
    .unwrap();
    let out = with_test_deps(|| {
        let mut keyboards = test_keyboards();
        apply_command_to(
            &mut snapshot,
            &command_json,
            &mut None,
            &mut keyboards,
            &mut None,
            &mut Vec::new(),
            &mut locale,
        )
        .expect("locale op applies")
    });
    assert_eq!(snapshot, before, "enumeration never touches the draft");
    assert_eq!(locale, Some(AppLanguage::Japanese));
    let payload: ViewPayload = out;
    assert_eq!(
        payload
            .sections
            .iter()
            .find(|section| section.id == "play")
            .expect("play exists")
            .label,
        "プレイ",
        "fresh locale re-projects immediately"
    );
    // Empty and missing tags fail loud instead of clearing silently.
    for bad in [
        serde_json::json!({
            "keyboards": [],
            "capable": true,
            "gamepads": [],
            "lang": "",
        }),
        serde_json::json!({ "keyboards": [], "capable": true }),
    ] {
        let bad_json = serde_json::to_string(&WireCommand {
            key: "device_truth".to_string(),
            index: 0,
            number: 0,
            op: WireOp::DeviceTruth,
            payload: bad,
        })
        .unwrap();
        with_test_deps(|| {
            let mut keyboards = test_keyboards();
            assert!(
                apply_command_to(
                    &mut snapshot,
                    &bad_json,
                    &mut None,
                    &mut keyboards,
                    &mut None,
                    &mut Vec::new(),
                    &mut locale,
                )
                .is_err(),
                "bad locale never persists"
            );
        });
    }
}

#[test]
fn locale_tag_mapping_round_trips() {
    // Ingestion and the save push share one mapping: `ja*` selects
    // Japanese, everything else falls back to English (the app only
    // ships the two).
    for (tag, want) in [
        ("ja", AppLanguage::Japanese),
        ("ja-JP", AppLanguage::Japanese),
        ("en", AppLanguage::English),
        ("en-US", AppLanguage::English),
        ("fr", AppLanguage::English),
    ] {
        let mapped = system_language_from_tag(tag);
        assert_eq!(mapped, want, "tag {tag} maps");
        assert_eq!(
            resolve_display(AppLanguage::SystemDefault, Some(mapped)).language(),
            want,
            "tag {tag} resolves for display"
        );
    }
    assert_eq!(app_language_tag(AppLanguage::Japanese), "ja");
    assert_eq!(app_language_tag(AppLanguage::English), "en");
    assert_eq!(app_language_tag(AppLanguage::SystemDefault), "");
}

#[test]
fn zero_based_top_level_hides_machine_names() {
    let (_snapshot, payload) = test_payload();
    // Top-level taxonomy is purpose-based: no machine names on top.
    let visible: Vec<&str> = payload
        .sections
        .iter()
        .filter(|section| section.visible)
        .map(|section| section.id.as_str())
        .collect();
    assert_eq!(
        visible,
        vec![
            "play",
            "controllers",
            "saves",
            "notifications",
            "app",
            "advanced"
        ],
        "visible categories are purpose-based"
    );
    for id in ["system.nes", "system.gbc", "system.gba"] {
        let machine = payload
            .sections
            .iter()
            .find(|section| section.id == id)
            .unwrap_or_else(|| panic!("{id} exists"));
        assert!(!machine.visible, "{id} hides under Advanced");
        assert!(
            machine.rows.iter().any(|row| row.kind == RowKind::Choice),
            "{id} keeps its fields"
        );
        assert!(
            machine.rows.iter().any(|row| row.kind == RowKind::Group
                && row.target == format!("input.{}", id.trim_start_matches("system."))),
            "{id} links to its controller screen"
        );
    }
}

#[test]
fn machine_sections_link_to_hidden_controller_screens() {
    let (_snapshot, payload) = test_payload();
    // Controller screens hold slots only; bindings moved to keys.*.
    let input = payload
        .sections
        .iter()
        .find(|section| section.id == "input.nes")
        .expect("controller screen exists");
    assert!(!input.visible, "controller screens hide from categories");
    assert!(
        input
            .rows
            .iter()
            .any(|row| row.key.starts_with("system.nes.slot.")),
        "slots live on the controller screen"
    );
    assert!(
        input.rows.iter().all(|row| row.kind != RowKind::Capture),
        "bindings moved out of the controller screen"
    );
    let keys = payload
        .sections
        .iter()
        .find(|section| section.id == "keys.nes")
        .expect("key mapping screen exists");
    assert!(!keys.visible, "key mapping screens hide from categories");
    assert!(
        keys.rows.iter().any(|row| row.kind == RowKind::Capture),
        "keyboard bindings are capturable rows"
    );
    assert!(
        payload
            .sections
            .iter()
            .all(|section| section.id != "shortcuts"),
        "no top-level shortcuts section on Android"
    );
    // Global shortcuts live one drill-down under Controllers, next to the
    // overlay detail screen: port-independent and always active.
    for (id, probe_key) in [
        ("controls.overlay_detail", "controls.overlay_opacity"),
        ("controls.shortcuts", "controls.shortcut.TogglePause"),
    ] {
        let hidden = payload
            .sections
            .iter()
            .find(|section| section.id == id)
            .unwrap_or_else(|| panic!("{id} screen exists"));
        assert!(!hidden.visible, "{id} hides from categories");
        assert!(
            hidden.rows.iter().any(|row| row.key == probe_key),
            "{id} holds its rows"
        );
        assert!(
            payload
                .sections
                .iter()
                .flat_map(|section| &section.rows)
                .any(|row| row.kind == RowKind::Group && row.target == id),
            "controllers links to {id}"
        );
    }
    let shortcuts = payload
        .sections
        .iter()
        .find(|section| section.id == "controls.shortcuts")
        .expect("shortcuts screen exists");
    assert_eq!(
        shortcuts.rows.len(),
        7,
        "desktop-only actions stay out: six phone shortcuts plus the screen reset"
    );
    assert!(
        shortcuts
            .rows
            .iter()
            .filter(|row| row.kind == RowKind::Capture)
            .count()
            == 6,
        "shortcuts rows are capturable"
    );
}

#[test]
fn desktop_only_shortcuts_stay_out_of_android() {
    let (_snapshot, payload) = test_payload();
    let shortcuts = payload
        .sections
        .iter()
        .find(|section| section.id == "controls.shortcuts")
        .expect("shortcuts screen exists");
    for banned in ["ToggleFullscreen", "OpenDebugger"] {
        assert!(
            !shortcuts.rows.iter().any(|row| row.target.contains(banned)),
            "{banned} is desktop-only"
        );
    }
}

#[test]
fn capture_rows_disable_without_input_device() {
    let (_snapshot, payload) = test_payload_with_keyboards(&[]);
    let captures: Vec<_> = payload
        .sections
        .iter()
        .flat_map(|section| &section.rows)
        .filter(|row| row.kind == RowKind::Capture)
        .collect();
    assert!(!captures.is_empty(), "capture rows exist to disable");
    for row in &captures {
        assert!(!row.enabled, "{} disables without an input device", row.key);
        assert!(!row.reason.is_empty(), "{} names the reason", row.key);
    }
    let (_snapshot, payload) = test_payload();
    assert!(
        payload
            .sections
            .iter()
            .flat_map(|section| &section.rows)
            .filter(|row| row.kind == RowKind::Capture)
            .all(|row| row.enabled),
        "keyboards present re-enable capture"
    );
}

#[test]
fn gamepad_alone_reopens_capture() {
    // Controller sources route to capture, so a connected pad suffices
    // even with no keyboard: pad buttons fall through when unmapped,
    // mapped keys (dpad) capture into the shared vocabulary.
    let gamepads = vec![GamepadInfo {
        name: "Pad".to_string(),
    }];
    let (_snapshot, payload) = with_test_deps(|| {
        let snapshot = default_snapshot();
        let json =
            open_payload(&snapshot, &[], None, &gamepads, None).expect("open payload builds");
        let payload: ViewPayload = json;
        (snapshot, payload)
    });
    let captures: Vec<_> = payload
        .sections
        .iter()
        .flat_map(|section| &section.rows)
        .filter(|row| row.kind == RowKind::Capture)
        .collect();
    assert!(!captures.is_empty(), "capture rows exist to enable");
    assert!(
        captures.iter().all(|row| row.enabled),
        "a connected pad re-enables capture"
    );
}

#[test]
fn start_capture_rejected_without_input_device() {
    let (mut snapshot, _) = test_payload_with_keyboards(&[]);
    let mut capture: CaptureSlot = None;
    let key = "controls.shortcut.TogglePause";
    let command_json = serde_json::to_string(&WireCommand {
        key: key.to_string(),
        index: 0,
        number: 0,
        op: WireOp::StartCapture,
        payload: serde_json::Value::Null,
    })
    .unwrap();
    let error = with_test_deps(|| {
        apply_command_to(
            &mut snapshot,
            &command_json,
            &mut capture,
            &mut Vec::new(),
            &mut None,
            &mut Vec::new(),
            &mut None,
        )
        .expect_err("start with no input device must fail")
    });
    assert!(
        error.contains("Input device"),
        "fail loud with the reason: {error}"
    );
    assert!(capture.is_none(), "no prompt strands");
}

#[test]
fn player_screens_wire_devices_types_and_summaries() {
    let (_snapshot, payload) = test_payload();
    // Default draft: built-in keyboard drives player 1.
    for (player_id, devices_id, types_id) in [
        ("player.1", "devices.p1", "types.1"),
        ("player.2", "devices.p2", "types.2"),
    ] {
        let player = payload
            .sections
            .iter()
            .find(|section| section.id == *player_id)
            .unwrap_or_else(|| panic!("{player_id} exists"));
        assert!(!player.visible, "{player_id} hides from categories");
        let devices_row = player
            .rows
            .iter()
            .find(|row| row.target == *devices_id)
            .expect("devices row exists");
        // The value line carries the port summary, so no supporting
        // text is needed (bare-row policy: self-evident stays bare).
        assert!(
            !devices_row.text.is_empty(),
            "devices row value carries the port summary"
        );
        let types_row = player
            .rows
            .iter()
            .find(|row| row.target == *types_id)
            .expect("controller type row exists");
        assert_eq!(types_row.text, "", "type row is a bare chevron");
        // Player rows on the controllers screen carry the same summary.
        let parent_row = payload
            .sections
            .iter()
            .flat_map(|section| &section.rows)
            .find(|row| row.target == *player_id)
            .expect("controllers links to the player screen");
        assert_eq!(
            parent_row.text, devices_row.text,
            "player summary matches the devices value"
        );
    }
    // Player 1 shows its built-in keyboard; player 2 is unassigned.
    let p1 = row(&payload, "controllers.player.1");
    assert!(
        p1.text.contains("Built-in Keyboard"),
        "p1 summary names the device"
    );
    assert_eq!(row(&payload, "controllers.player.2").text, "Not assigned");
    // Types screens fan out per machine with a player-1 row everywhere
    // and a player-2 row on NES only.
    assert_eq!(
        payload
            .sections
            .iter()
            .find(|section| section.id == "types.1")
            .expect("types.1 exists")
            .rows
            .len(),
        3,
        "every machine has a player-1 slot"
    );
    assert_eq!(
        payload
            .sections
            .iter()
            .find(|section| section.id == "types.2")
            .expect("types.2 exists")
            .rows
            .len(),
        1,
        "only NES has a player-2 slot"
    );
}

#[test]
fn devices_screen_lists_keyboards_with_membership_switches() {
    let (_snapshot, payload) = test_payload_with_keyboards(&[]);
    let devices = payload
        .sections
        .iter()
        .find(|section| section.id == "devices.p1")
        .expect("devices screen exists");
    assert!(
        devices.rows.iter().any(|row| row.key == "devices.p1.none"),
        "empty enumeration explains itself instead of rendering blank"
    );
    let (_snapshot, payload) = test_payload();
    let devices = payload
        .sections
        .iter()
        .find(|section| section.id == "devices.p1")
        .expect("devices screen exists");
    let builtin = devices
        .rows
        .iter()
        .find(|row| row.key == "devices.p1.kbd:builtin")
        .expect("built-in keyboard row exists");
    assert_eq!(builtin.kind, RowKind::Toggle);
    assert_eq!(
        builtin.selected, 1,
        "built-in keyboard drives p1 by default"
    );
    assert_eq!(builtin.detail_target, "device.kbd:builtin");
    let usb = devices
        .rows
        .iter()
        .find(|row| row.key == "devices.p1.kbd:usb-1")
        .expect("external keyboard row exists");
    assert_eq!(usb.selected, 0, "connected keyboards start unassigned");
    // Every enumerated keyboard gets a detail screen with status plus
    // per-system key mapping links.
    for member in ["kbd:builtin", "kbd:usb-1"] {
        let detail = payload
            .sections
            .iter()
            .find(|section| section.id == format!("device.{member}"))
            .unwrap_or_else(|| panic!("device screen for {member} exists"));
        assert!(!detail.visible, "device screens hide from categories");
        for system in ["nes", "gbc", "gba"] {
            assert!(
                detail
                    .rows
                    .iter()
                    .any(|row| row.kind == RowKind::Group
                        && row.target == format!("keys.{system}")),
                "{member} links to the {system} key table"
            );
        }
    }
}

#[test]
fn device_toggle_flips_port_membership() {
    let (mut snapshot, _) = test_payload();
    let mut keyboards = test_keyboards();
    // USB keyboard joins player 2: switch on.
    let on = apply(
        &mut snapshot,
        &mut None,
        &mut keyboards,
        "devices.p2.kbd:usb-1",
        WireOp::Set,
        1,
        0,
    );
    assert_eq!(
        row(&on, "devices.p2.kbd:usb-1").selected,
        1,
        "switch reflects membership"
    );
    assert!(
        snapshot
            .app_state
            .port_devices
            .get("p2")
            .is_some_and(|set| set.iter().any(|member| member == "kbd:usb-1")),
        "membership persists in the draft"
    );
    assert!(
        row(&on, "controllers.player.2")
            .text
            .contains("USB Keyboard"),
        "player summary follows membership"
    );
    // And leaves again: switch off prunes the member id.
    let off = apply(
        &mut snapshot,
        &mut None,
        &mut keyboards,
        "devices.p2.kbd:usb-1",
        WireOp::Set,
        0,
        0,
    );
    assert_eq!(
        row(&off, "controllers.player.2").text,
        "Not assigned",
        "empty port reads Not assigned"
    );
}

#[test]
fn phantom_builtin_never_surfaces_without_hardware() {
    // Phones and tablets enumerate no built-in keyboard, yet the
    // shared default drafts p1=[kbd:builtin]. Summaries and counts
    // must read empty, and the first fresh enumeration prunes it.
    let (mut snapshot, _) = test_payload_with_keyboards(&[]);
    assert_eq!(
        row(&test_payload_with_keyboards(&[]).1, "controllers.player.1").text,
        "Not assigned",
        "no hardware, no phantom controller name"
    );
    let mut keyboards = Vec::new();
    let command_json = serde_json::to_string(&WireCommand {
        key: "device_truth".to_string(),
        index: 0,
        number: 0,
        op: WireOp::DeviceTruth,
        payload: serde_json::json!({
            "keyboards": [{ "member": "kbd:usb-1", "name": "USB Keyboard", "builtin": false }],
            "capable": true,
            "gamepads": [],
        }),
    })
    .unwrap();
    with_test_deps(|| {
        apply_command_to(
            &mut snapshot,
            &command_json,
            &mut None,
            &mut keyboards,
            &mut None,
            &mut Vec::new(),
            &mut None,
        )
        .expect("keyboards op applies")
    });
    assert!(
        snapshot
            .app_state
            .port_devices
            .get("p1")
            .is_none_or(|set| !set.iter().any(|member| member == "kbd:builtin")),
        "builtin membership pruned without hardware"
    );
    // …while an unplugged USB descriptor keeps its assignment so a
    // reconnect restores it without re-setup.
    snapshot
        .app_state
        .port_devices
        .entry("p1".to_string())
        .or_default()
        .push("kbd:usb-1".to_string());
    let off = with_test_deps(|| {
        apply_command_to(
            &mut snapshot,
            &serde_json::to_string(&WireCommand {
                key: "device_truth".to_string(),
                index: 0,
                number: 0,
                op: WireOp::DeviceTruth,
                payload: serde_json::json!({ "keyboards": [], "capable": true, "gamepads": [] }),
            })
            .unwrap(),
            &mut None,
            &mut Vec::new(),
            &mut None,
            &mut Vec::new(),
            &mut None,
        )
        .expect("empty enumeration applies")
    });
    let payload: ViewPayload = off;
    assert!(
        snapshot
            .app_state
            .port_devices
            .get("p1")
            .is_some_and(|set| set.iter().any(|member| member == "kbd:usb-1")),
        "USB assignment survives unplug"
    );
    assert_eq!(
        row(&payload, "controllers.player.1").text,
        "Not assigned",
        "unplugged USB reads empty like the device list, assignment kept"
    );
}

#[test]
fn keyboards_op_upserts_enumeration_without_mutating_draft() {
    let (mut snapshot, _) = test_payload_with_keyboards(&[]);
    let before = snapshot.clone();
    let mut keyboards = Vec::new();
    let command_json = serde_json::to_string(&WireCommand {
        key: "device_truth".to_string(),
        index: 0,
        number: 0,
        op: WireOp::DeviceTruth,
        payload: serde_json::json!({
            "keyboards": serde_json::to_value(test_keyboards()).unwrap(),
            "capable": true,
            "gamepads": [],
        }),
    })
    .unwrap();
    let out = with_test_deps(|| {
        apply_command_to(
            &mut snapshot,
            &command_json,
            &mut None,
            &mut keyboards,
            &mut None,
            &mut Vec::new(),
            &mut None,
        )
        .expect("keyboards op applies")
    });
    assert_eq!(snapshot, before, "enumeration never touches the draft");
    assert_eq!(keyboards.len(), 2, "vault holds the enumerated keyboards");
    let payload: ViewPayload = out;
    assert!(
        payload
            .sections
            .iter()
            .flat_map(|section| &section.rows)
            .any(|row| row.key == "devices.p1.kbd:usb-1"),
        "refreshed payload lists the enumerated keyboard"
    );
}

fn test_gamepads() -> Vec<GamepadInfo> {
    vec![GamepadInfo {
        name: "Wireless Controller".to_string(),
    }]
}

#[test]
fn gamepads_op_upserts_without_mutating_draft() {
    let (mut snapshot, _) = test_payload();
    let before = snapshot.clone();
    let mut gamepads = Vec::new();
    let command_json = serde_json::to_string(&WireCommand {
        key: "device_truth".to_string(),
        index: 0,
        number: 0,
        op: WireOp::DeviceTruth,
        payload: serde_json::json!({
            "keyboards": serde_json::to_value(test_keyboards()).unwrap(),
            "capable": true,
            "gamepads": serde_json::to_value(test_gamepads()).unwrap(),
        }),
    })
    .unwrap();
    let out = with_test_deps(|| {
        apply_command_to(
            &mut snapshot,
            &command_json,
            &mut None,
            &mut test_keyboards(),
            &mut None,
            &mut gamepads,
            &mut None,
        )
        .expect("gamepads op applies")
    });
    assert_eq!(snapshot, before, "enumeration never touches the draft");
    assert_eq!(gamepads.len(), 1, "vault holds the counted gamepads");
    let payload: ViewPayload = out;
    let status = row(&payload, "controls.auto_hide_status");
    assert!(
        status.text.contains("Wireless Controller"),
        "status names the counted gamepad: {}",
        status.text
    );
    let bad = serde_json::to_string(&WireCommand {
        key: "device_truth".to_string(),
        index: 0,
        number: 0,
        op: WireOp::DeviceTruth,
        payload: serde_json::json!({
            "keyboards": [],
            "capable": true,
            "gamepads": [{ "name": "" }],
        }),
    })
    .unwrap();
    with_test_deps(|| {
        assert!(
            apply_command_to(
                &mut snapshot,
                &bad,
                &mut None,
                &mut test_keyboards(),
                &mut None,
                &mut gamepads,
                &mut None,
            )
            .is_err(),
            "nameless gamepads never persist"
        );
    });
}

#[test]
fn auto_hide_status_row_shows_none_when_empty() {
    let (_, payload) = test_payload();
    let status = row(&payload, "controls.auto_hide_status");
    assert_eq!(
        status.text,
        I18n::new(AppLanguage::English).text(UiText::NotAssigned),
        "empty gate says so"
    );
}

fn touch_payload(capable: Option<bool>) -> ViewPayload {
    let (mut snapshot, _) = test_payload();
    if let Some(value) = capable {
        let command_json = serde_json::to_string(&WireCommand {
            key: "device_truth".to_string(),
            index: 0,
            number: 0,
            op: WireOp::DeviceTruth,
            payload: serde_json::json!({
                "keyboards": serde_json::to_value(test_keyboards()).unwrap(),
                "capable": value,
                "gamepads": [],
            }),
        })
        .unwrap();
        let mut touch = None;
        let out = with_test_deps(|| {
            apply_command_to(
                &mut snapshot,
                &command_json,
                &mut None,
                &mut test_keyboards(),
                &mut touch,
                &mut Vec::new(),
                &mut None,
            )
            .expect("touch op applies")
        });
        assert_eq!(touch, capable, "vault records touch capability");
        return out;
    }
    // Unknown capability (old client, unit default): no push arrives,
    // rows behave as before.
    let touch = None;
    with_test_deps(|| {
        open_payload(&snapshot, &test_keyboards(), touch, &[], None).expect("open payload builds")
    })
}

#[test]
fn touchless_terminal_disables_overlay_choices() {
    let payload = touch_payload(Some(false));
    let visibility = row(&payload, "controls.overlay_visibility");
    assert_eq!(
        visibility.choices.len(),
        3,
        "Always/Auto/Hidden stay listed"
    );
    for choice in visibility.choices.iter().take(2) {
        assert!(!choice.enabled, "Always/Auto are not enableable");
        assert!(
            !choice.reason.is_empty(),
            "disabled choice explains the missing touch screen"
        );
    }
    assert!(
        visibility.choices[2].enabled,
        "Hidden stays selectable as the only honest value"
    );
}

#[test]
fn touch_capable_terminal_keeps_overlay_choices() {
    for capable in [None, Some(true)] {
        let payload = touch_payload(capable);
        let visibility = row(&payload, "controls.overlay_visibility");
        assert!(
            visibility.choices.iter().all(|choice| choice.enabled),
            "unknown/touch terminals keep every overlay choice"
        );
    }
}

#[test]
fn reset_section_restores_screen_defaults() {
    let (mut snapshot, _) = test_payload();
    let mut keyboards = test_keyboards();
    // Dirty the volume, then reset the play screen.
    set(&mut snapshot, "audio.volume", 0, 77);
    assert_eq!(snapshot.local.audio.master_volume_percent, 77);
    let reset = serde_json::to_string(&WireCommand {
        key: "reset.play".to_string(),
        index: 0,
        number: 0,
        op: WireOp::ResetSection,
        payload: serde_json::Value::Null,
    })
    .unwrap();
    let out = with_test_deps(|| {
        apply_command_to(
            &mut snapshot,
            &reset,
            &mut None,
            &mut keyboards,
            &mut None,
            &mut Vec::new(),
            &mut None,
        )
        .expect("reset applies")
    });
    let payload: ViewPayload = out;
    assert_eq!(
        row(&payload, "audio.volume").number,
        i64::from(
            HostBackendLocalSettings::default()
                .audio
                .master_volume_percent
        ),
        "volume returns to default"
    );
}
#[test]
fn every_emitted_reset_key_executes() {
    // Device-screen resets used index ids (`reset.devices.1`) while the
    // reset arm matched port ids (`p1`): every such scope errored. Press
    // each emitted reset key so a namespace drift fails loud here.
    let (mut snapshot, payload) = test_payload();
    let mut keyboards = test_keyboards();
    let keys: Vec<String> = payload
        .sections
        .iter()
        .flat_map(|section| &section.rows)
        .filter(|row| row.kind == RowKind::Button)
        .map(|row| row.key.clone())
        .collect();
    assert!(!keys.is_empty(), "reset rows exist to press");
    for key in keys {
        let reset = serde_json::to_string(&WireCommand {
            key: key.clone(),
            index: 0,
            number: 0,
            op: WireOp::ResetSection,
            payload: serde_json::Value::Null,
        })
        .unwrap();
        with_test_deps(|| {
            apply_command_to(
                &mut snapshot,
                &reset,
                &mut None,
                &mut keyboards,
                &mut None,
                &mut Vec::new(),
                &mut None,
            )
        })
        .unwrap_or_else(|_| panic!("reset executes: {key}"));
    }
}

#[test]
fn same_profile_drives_both_slots() {
    let (mut snapshot, payload) = test_payload();
    // The standard pad is one instance per slot: P1 taking it must not
    // lock P2 out. (Profile-id suppression used to forbid exactly this
    // sharing; multi-port coverage is the only real conflict.)
    let p1_key = "system.nes.slot.nes.attachment.player1";
    let p2_key = "system.nes.slot.nes.attachment.player2";
    let pad = row(&payload, p1_key)
        .choices
        .iter()
        .find(|choice| {
            choice
                .value
                .as_str()
                .is_some_and(|value| value.contains("standard"))
        })
        .map(|choice| choice.value.clone())
        .expect("standard pad listed for player1");
    let index = row(&payload, p1_key)
        .choices
        .iter()
        .position(|choice| choice.value == pad)
        .expect("pad index resolves");
    let next = set(&mut snapshot, p1_key, index, 0);
    let p2 = row(&next, p2_key);
    let shared = p2
        .choices
        .iter()
        .find(|choice| choice.value == pad)
        .expect("same pad listed for player2");
    assert!(shared.enabled, "P2 keeps its own standard pad");
    assert!(
        next.issues.is_empty(),
        "shared single-port profiles stay saveable: {:?}",
        next.issues
    );
}

#[test]
fn covered_slot_disables_concrete_controllers() {
    let (mut snapshot, payload) = test_payload();
    let p1_key = "system.nes.slot.nes.attachment.player1";
    let p2_key = "system.nes.slot.nes.attachment.player2";
    // The famicom set physically covers both slots; P2 then offers
    // nothing but "not assigned" until P1 releases it.
    let famicom = row(&payload, p1_key)
        .choices
        .iter()
        .find(|choice| {
            choice
                .value
                .as_str()
                .is_some_and(|value| value.contains("famicom"))
        })
        .map(|choice| choice.value.clone())
        .expect("famicom set listed for player1");
    let index = row(&payload, p1_key)
        .choices
        .iter()
        .position(|choice| choice.value == famicom)
        .expect("famicom index resolves");
    let next = set(&mut snapshot, p1_key, index, 0);
    let covered = row(&next, p2_key);
    for choice in &covered.choices {
        if choice.value.is_null() {
            assert!(choice.enabled, "not-assigned stays pickable");
        } else {
            assert!(!choice.enabled, "covered slot refuses controllers");
            assert!(
                !choice.reason.is_empty(),
                "disabled choice names the covering slot"
            );
        }
    }
    // Drafting the impossible combo anyway (tampered client, old
    // file) fails the finish probe loudly instead of saving silence.
    let pad = covered
        .choices
        .iter()
        .position(|choice| {
            choice
                .value
                .as_str()
                .is_some_and(|value| value.contains("standard"))
        })
        .expect("standard pad listed for player2");
    let conflicted = set(&mut snapshot, p2_key, pad, 0);
    assert!(
        conflicted
            .issues
            .iter()
            .any(|issue| issue.contains("conflict")),
        "finish probe reports the slot conflict: {:?}",
        conflicted.issues
    );
}

#[test]
fn capture_round_trips_through_hardware_keys() {
    let (mut snapshot, _) = test_payload();
    let mut capture: CaptureSlot = None;
    // Find any binding capture row on the NES controller screen.
    let target_key = with_test_deps(|| {
        let vm = build_vm(
            &snapshot,
            &test_deps(),
            resolve_display(AppLanguage::SystemDefault, None),
        )
        .expect("vm builds");
        let keyboards = test_keyboards();
        let truth = projection(
            &keyboards,
            None,
            &[],
            resolve_display(AppLanguage::SystemDefault, None),
        );
        let sections = build_sections(&vm, &snapshot, truth);
        sections
            .iter()
            .flat_map(|section| &section.rows)
            .find(|row| row.kind == RowKind::Capture)
            .map(|row| (row.key.clone(), row.target.clone()))
            .expect("capture row exists")
    });
    let (key, _) = target_key;
    let started = apply(
        &mut snapshot,
        &mut capture,
        &mut test_keyboards(),
        &key,
        WireOp::StartCapture,
        0,
        0,
    );
    let pending = started.capture.expect("capture pending");
    assert!(!pending.prompt.is_empty());
    // Unmapped keycodes and releases must not consume: the platform
    // keeps handling them (volume, Back, ...) while capture stays open.
    let (_, consumed_release) = with_test_deps(|| {
        apply_hardware_key(
            29,
            false,
            &mut snapshot,
            &mut capture,
            projection(
                &test_keyboards(),
                None,
                &[],
                resolve_display(AppLanguage::SystemDefault, None),
            ),
        )
        .expect("hardware key release probes")
    });
    assert!(!consumed_release, "release is not consumed");
    assert!(capture.is_some(), "capture stays pending on release");
    let (_, consumed_unknown) = with_test_deps(|| {
        apply_hardware_key(
            999,
            true,
            &mut snapshot,
            &mut capture,
            projection(
                &test_keyboards(),
                None,
                &[],
                resolve_display(AppLanguage::SystemDefault, None),
            ),
        )
        .expect("unknown keycode probes")
    });
    assert!(!consumed_unknown, "unmapped keycode is not consumed");
    assert!(capture.is_some(), "capture stays pending on unknown key");
    let (_, consumed) = with_test_deps(|| {
        apply_hardware_key(
            29,
            true,
            &mut snapshot,
            &mut capture,
            projection(
                &test_keyboards(),
                None,
                &[],
                resolve_display(AppLanguage::SystemDefault, None),
            ),
        )
        .expect("hardware key applies")
    });
    assert!(consumed, "mapped press is consumed");
    assert!(capture.is_none(), "capture consumes after one key");
}

#[test]
fn clear_binding_unbinds_without_keyboard() {
    let (mut snapshot, _) = test_payload();
    let mut capture: CaptureSlot = None;
    let key = with_test_deps(|| {
        let vm = build_vm(
            &snapshot,
            &test_deps(),
            resolve_display(AppLanguage::SystemDefault, None),
        )
        .expect("vm builds");
        let keyboards = test_keyboards();
        let truth = projection(
            &keyboards,
            None,
            &[],
            resolve_display(AppLanguage::SystemDefault, None),
        );
        let sections = build_sections(&vm, &snapshot, truth);
        sections
            .iter()
            .flat_map(|section| &section.rows)
            .find(|row| row.kind == RowKind::Capture)
            .map(|row| row.key.clone())
            .expect("capture row exists")
    });
    let cleared = apply(
        &mut snapshot,
        &mut capture,
        &mut test_keyboards(),
        &key,
        WireOp::ClearBinding,
        0,
        0,
    );
    let _ = cleared;
    assert!(capture.is_none());
}

#[test]
fn number_rows_advertise_vm_accepted_bounds() {
    let (mut snapshot, _) = test_payload();
    let bounds: Vec<(String, i64, i64)> = with_test_deps(|| {
        let vm = build_vm(
            &snapshot,
            &test_deps(),
            resolve_display(AppLanguage::SystemDefault, None),
        )
        .expect("vm builds");
        let keyboards = test_keyboards();
        let truth = projection(
            &keyboards,
            None,
            &[],
            resolve_display(AppLanguage::SystemDefault, None),
        );
        let sections = build_sections(&vm, &snapshot, truth);
        sections
            .iter()
            .flat_map(|section| &section.rows)
            .filter(|row| row.kind == RowKind::Number)
            .map(|row| (row.key.clone(), row.min, row.max))
            .collect()
    });
    assert!(!bounds.is_empty());
    for (key, min, max) in bounds {
        assert!(min < max, "{key}: bounds order");
        set(&mut snapshot, &key, 0, min);
        set(&mut snapshot, &key, 0, max);
    }
}

#[test]
fn empty_slots_surface_live_issues() {
    let (mut snapshot, payload) = test_payload();
    for slot in [
        "system.nes.slot.nes.attachment.player1",
        "system.nes.slot.nes.attachment.player2",
    ] {
        let none = row(&payload, slot)
            .choices
            .iter()
            .position(|choice| choice.value.is_null())
            .expect("unassigned choice exists");
        set(&mut snapshot, slot, none, 0);
    }
    let probed = with_test_deps(|| {
        let vm = build_vm(
            &snapshot,
            &test_deps(),
            resolve_display(AppLanguage::SystemDefault, None),
        )
        .expect("vm builds");
        draft_issues(&vm, resolve_display(AppLanguage::SystemDefault, None))
    });
    assert!(
        probed.iter().any(|issue| issue.contains("controller")),
        "all-None slots explain themselves: {probed:?}"
    );
    // Japanese resolves the same kind through the localized template.
    let japanese = with_test_deps(|| {
        let vm = build_vm(
            &snapshot,
            &test_deps(),
            resolve_display(AppLanguage::Japanese, None),
        )
        .expect("vm builds");
        draft_issues(&vm, resolve_display(AppLanguage::Japanese, None))
    });
    assert!(
        japanese
            .iter()
            .any(|issue| issue.contains("コントローラー")),
        "no-controller explains itself in Japanese: {japanese:?}"
    );
}

#[test]
fn same_edge_hides_status_area_mode() {
    let (mut snapshot, payload) = test_payload();
    // Defaults (BottomLeft vs TopLeft) show the row.
    assert!(row(&payload, "status.area_mode").visible);
    // Move status to the bottom edge: the row hides.
    let status = row(&payload, "status.position");
    let bottom_right = status
        .choices
        .iter()
        .position(|choice| {
            let label = choice.label.to_lowercase();
            label.contains("bottom") && label.contains("right")
        })
        .expect("bottom-right choice exists");
    let next = set(&mut snapshot, "status.position", bottom_right, 0);
    assert!(
        !row(&next, "status.area_mode").visible,
        "same-edge rule hides the row"
    );
}

#[test]
fn bare_rows_are_reviewed_allowlist() {
    // Descriptions only where they add information: self-evident
    // labels stay bare (no AI-slop restatements). This allowlist pins
    // the policy; a new bare row fails here until it is reviewed.
    // - reset.*: the label states the action, the confirm dialog states
    //   the consequences.
    // - sliders with inline values (volume, opacity, scale, offset,
    //   rumble strength): the value line already says it.
    // - color/position/target choice rows: the choices list the options.
    // - haptics/status toggles: label plus on/off is the whole story.
    // - app.about, machine drill-down rows (advanced.*), controller-type
    //   rows (player.*.type): standard navigation pattern, the label
    //   plus chevron says where it goes.
    // - per-machine settings (system.*) and the controller links
    //   (system.*.controllers): label plus choice value, or label plus
    //   chevron in machine context, is self-contained.
    // - player.*.devices: the value line carries the port summary.
    // - mute/vsync toggles and per-device key-mapping links: self-evident
    //   labels with no restatement (on/off, or the machine context).
    let mut bare: Vec<String> = payload_bare_keys(&test_payload().1);
    bare.sort();
    let mut expected: Vec<String> = vec![
        "advanced.gba",
        "advanced.gbc",
        "advanced.nes",
        "app.about",
        "audio.muted",
        "audio.volume",
        "general.language",
        "controls.cartridge_rumble_strength",
        "controls.cartridge_rumble_target",
        "controls.overlay_haptics",
        "controls.overlay_opacity",
        "controls.overlay_scale",
        "controls.overlay_vertical_offset",
        "notifications.error_color",
        "notifications.info_color",
        "notifications.info_duration",
        "notifications.warning_color",
        "notifications.position",
        "notifications.warning_duration",
        "player.1.devices",
        "player.1.type",
        "player.2.devices",
        "player.2.type",
        "status.enabled",
        "status.position",
        "system.gba.controllers",
        "system.gba.peripheral.solar_light",
        "system.gbc.controllers",
        "system.gbc.system.hardware_model",
        "system.gbc.system.rtc_sync",
        "system.nes.controllers",
        "system.nes.core.mmc3_irq_variant",
        "system.nes.video.filter",
        "video.vsync",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    for prefix in [
        "reset.all",
        "reset.audio.detail",
        "reset.controllers",
        "reset.controls.overlay_detail",
        "reset.controls.shortcuts",
        "reset.devices.p1",
        "reset.devices.p2",
        "reset.input.gba",
        "reset.input.gbc",
        "reset.input.nes",
        "reset.keys.gba",
        "reset.keys.gbc",
        "reset.keys.nes",
        "reset.notifications",
        "reset.play",
        "reset.saves",
        "reset.system.gba",
        "reset.system.gbc",
        "reset.system.nes",
    ] {
        expected.push(prefix.to_string());
    }
    // Per-device key-mapping links (`device.<member>.keys.<system>`): the
    // machine context is self-contained, so no description. Members and
    // systems are dynamic, hence generated, not listed above.
    for member in ["kbd:builtin", "kbd:usb-1"] {
        for system in ["gba", "gbc", "nes"] {
            expected.push(format!("device.{member}.keys.{system}"));
        }
    }
    expected.sort_unstable();
    assert_eq!(bare, expected, "bare rows changed: review the new policy");
}

#[test]
fn revert_to_original_leaves_draft_untouched() {
    // Dirty is draft-vs-pristine: a flip-flop must read clean so the
    // screen never asks to save nothing.
    let (mut snapshot, _) = test_payload();
    let pristine = snapshot.clone();
    set(&mut snapshot, "audio.volume", 0, 77);
    assert_ne!(snapshot, pristine, "edits still dirty the draft");
    set(
        &mut snapshot,
        "audio.volume",
        0,
        i64::from(
            HostBackendLocalSettings::default()
                .audio
                .master_volume_percent,
        ),
    );
    assert_eq!(snapshot, pristine, "revert reads clean");
}

#[test]
fn clearing_an_empty_binding_leaves_draft_untouched() {
    // Unassigned shortcut + Clear: the second clear hits an empty binding,
    // so the VM round-trips nothing and the draft must not move (no
    // phantom dirty).
    let (mut snapshot, payload) = test_payload();
    let key = payload
        .sections
        .iter()
        .flat_map(|section| &section.rows)
        .find(|row| row.kind == RowKind::Capture)
        .map(|row| row.key.clone())
        .expect("capture row exists");
    let mut capture: CaptureSlot = None;
    let mut keyboards = test_keyboards();
    apply(
        &mut snapshot,
        &mut capture,
        &mut keyboards,
        &key,
        WireOp::ClearBinding,
        0,
        0,
    );
    let cleared_once = snapshot.clone();
    apply(
        &mut snapshot,
        &mut capture,
        &mut keyboards,
        &key,
        WireOp::ClearBinding,
        0,
        0,
    );
    assert_eq!(snapshot, cleared_once, "empty clear reads clean");
}

#[test]
fn toggle_and_number_commands_round_trip() {
    let (mut snapshot, _) = test_payload();
    let muted = set(&mut snapshot, "audio.muted", 1, 0);
    assert!(snapshot.local.audio.muted);
    assert_eq!(row(&muted, "audio.muted").selected, 1);

    let volume = set(&mut snapshot, "audio.volume", 0, 42);
    assert_eq!(snapshot.local.audio.master_volume_percent, 42);
    assert_eq!(row(&volume, "audio.volume").number, 42);
}

#[test]
fn choice_command_updates_language_and_system_field() {
    let (mut snapshot, payload) = test_payload();
    let language_row = row(&payload, "general.language");
    let japanese = language_row
        .choices
        .iter()
        .position(|choice| choice.value == serde_json::json!("japanese"))
        .expect("japanese choice exists");
    let next = set(&mut snapshot, "general.language", japanese, 0);
    assert_eq!(snapshot.shared.general.language, AppLanguage::Japanese);

    let filter_row = row(&next, "system.nes.video.filter");
    let rgb = filter_row
        .choices
        .iter()
        .position(|choice| choice.value == serde_json::json!("ntsc_rgb"))
        .expect("ntsc_rgb choice exists");
    let _filter = set(&mut snapshot, "system.nes.video.filter", rgb, 0);
    let nes = snapshot
        .shared
        .systems
        .get(NesFactory.system_id().as_ref())
        .map(|settings| settings.downcast_ref::<NesSettings>().unwrap().video.filter)
        .unwrap();
    assert_eq!(nes, NesVideoFilter::NtscRgb);
}

#[test]
fn unknown_keys_and_indices_fail_without_mutation() {
    let (mut snapshot, _) = test_payload();
    let before = snapshot.clone();
    with_test_deps(|| {
        let bad_key = serde_json::to_string(&WireCommand {
            key: "nope.missing".to_string(),
            index: 0,
            number: 0,
            op: WireOp::Set,
            payload: serde_json::Value::Null,
        })
        .unwrap();
        assert!(
            apply_command_to(
                &mut snapshot,
                &bad_key,
                &mut None,
                &mut test_keyboards(),
                &mut None,
                &mut Vec::new(),
                &mut None
            )
            .is_err()
        );

        let bad_index = serde_json::to_string(&WireCommand {
            key: "audio.sample_rate".to_string(),
            index: 99,
            number: 0,
            op: WireOp::Set,
            payload: serde_json::Value::Null,
        })
        .unwrap();
        assert!(
            apply_command_to(
                &mut snapshot,
                &bad_index,
                &mut None,
                &mut test_keyboards(),
                &mut None,
                &mut Vec::new(),
                &mut None
            )
            .is_err()
        );

        // Unknown wire ops fail at the parse boundary (the op enum is
        // exhaustive, so dispatch itself cannot see them).
        let bad_op =
            r#"{"key":"audio.volume","index":0,"number":42,"op":"frobnicate","payload":null}"#;
        assert!(
            apply_command_to(
                &mut snapshot,
                bad_op,
                &mut None,
                &mut test_keyboards(),
                &mut None,
                &mut Vec::new(),
                &mut None
            )
            .is_err()
        );

        let bad_port = serde_json::to_string(&WireCommand {
            key: "devices.evil.kbd:builtin".to_string(),
            index: 1,
            number: 0,
            op: WireOp::Set,
            payload: serde_json::Value::Null,
        })
        .unwrap();
        assert!(
            apply_command_to(
                &mut snapshot,
                &bad_port,
                &mut None,
                &mut test_keyboards(),
                &mut None,
                &mut Vec::new(),
                &mut None
            )
            .is_err(),
            "unknown ports never persist"
        );

        let bad_scope = serde_json::to_string(&WireCommand {
            key: "reset.evil".to_string(),
            index: 0,
            number: 0,
            op: WireOp::ResetSection,
            payload: serde_json::Value::Null,
        })
        .unwrap();
        assert!(
            apply_command_to(
                &mut snapshot,
                &bad_scope,
                &mut None,
                &mut test_keyboards(),
                &mut None,
                &mut Vec::new(),
                &mut None
            )
            .is_err(),
            "unknown reset scopes never no-op silently"
        );
    });
    assert_eq!(snapshot, before);
}

#[test]
fn finish_rejects_conflicting_bindings() {
    // `finish` gates on the shared validator: an invalid draft fails.
    // Install the process-wide deps explicitly: `finish_snapshot` reads
    // them instead of taking per-call copies, so this test must not
    // rely on another test's installation.
    with_test_deps(|| {
        let mut snapshot = default_snapshot();
        snapshot.local.audio.master_volume_percent = 100;
        assert!(finish_snapshot(&snapshot, None).is_ok());
        snapshot.local.audio.latency_ms = 5;
        assert!(finish_snapshot(&snapshot, None).is_err());
    });
}

//! Android settings projection: view-model DTOs rendered into wire sections.
//!
//! Every builder here is pure: it reads the view model, the draft snapshot,
//! and the vault device truth, and returns wire rows. No mutation.

use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_gui_viewmodel::settings::SettingsViewModel;
use nerust_settings_core::{
    editor::CaptureTarget,
    i18n::UiText,
    input::{BUILTIN_KEYBOARD_MEMBER, member_is_gamepad, member_is_shareable},
    model::{
        app_state::{PORT_PLAYER_1, PORT_PLAYER_2},
        input::ShortcutAction,
        local::{
            NotificationColor, NotificationPosition, RumbleTarget, ScreenOrientation,
            TouchOverlayVisibility,
        },
    },
};

use super::wire::{
    CaptureSlot, CaptureWire, GamepadInfo, I18n, KeyboardInfo, Projection, RowWire, SectionWire,
    ViewPayload, VmDeps, build_vm, button_row, capture_open, capture_row, choice_row, draft_issues,
    encode_target, group_row, header_row, info_row, number_row, target_system, toggle_row,
};
use crate::bridge_contract::SETTINGS_SCHEMA_VERSION;

/// Assemble the wire payload from an already-built view model: sections,
/// live issues from a non-fatal `finish` probe, and the pending capture
/// state. The single projection site: every command arm funnels here.
pub(super) fn project_payload(
    vm: &SettingsViewModel,
    snapshot: &SettingsSnapshot,
    deps: &VmDeps,
    capture: &CaptureSlot,
    truth: Projection,
) -> ViewPayload {
    ViewPayload {
        schema_version: SETTINGS_SCHEMA_VERSION,
        sections: build_sections(vm, snapshot, truth),
        issues: draft_issues(vm, truth.i18n),
        capture: capture.as_ref().map(|(target, prompt)| CaptureWire {
            target: encode_target(target, &target_system(target, deps)),
            prompt: prompt.clone(),
        }),
    }
}

/// Project after a snapshot-direct mutation (device truth, reset, language):
/// build the view model exactly once, then project once.
pub(super) fn project_fresh(
    snapshot: &SettingsSnapshot,
    deps: &VmDeps,
    capture: &CaptureSlot,
    truth: Projection,
) -> Result<ViewPayload, String> {
    let vm = build_vm(snapshot, deps, truth.i18n)?;
    Ok(project_payload(&vm, snapshot, deps, capture, truth))
}

/// Global player ports (not per machine): every system's player-N slot
/// fans out from the same port set. Screen ids derive from the port index,
/// so the table holds port ids only.
pub(super) const PLAYER_PORTS: [&str; 2] = [PORT_PLAYER_1, PORT_PLAYER_2];

fn player_id(index: usize) -> String {
    format!("player.{}", index + 1)
}

fn devices_id(port: &str) -> String {
    format!("devices.{port}")
}

fn types_id(index: usize) -> String {
    format!("types.{}", index + 1)
}

fn player_label(index: usize, i18n: I18n) -> String {
    i18n.text(if index == 0 {
        UiText::PlayerOne
    } else {
        UiText::PlayerTwo
    })
}

pub(super) fn build_sections(
    vm: &SettingsViewModel,
    snapshot: &SettingsSnapshot,
    truth: Projection,
) -> Vec<SectionWire> {
    let i18n = truth.i18n;
    let mut sections = vec![
        build_play_section(vm, snapshot, i18n),
        build_saves_section(vm, i18n),
        build_notifications_section(vm, i18n),
        build_app_section(vm, snapshot, i18n),
        build_advanced_section(vm, snapshot, i18n),
        build_about_section(i18n),
        build_audio_detail_section(vm, i18n),
    ];
    sections.extend(build_controllers_sections(
        vm,
        snapshot,
        i18n,
        truth.keyboards,
        truth.touch_capable,
        truth.gamepads,
    ));
    sections.extend(build_player_sections(
        vm,
        snapshot,
        i18n,
        truth.keyboards,
        truth.gamepads,
    ));
    sections.extend(build_system_sections(vm, i18n));
    sections.extend(build_input_sections(vm, i18n));
    sections.extend(build_keys_sections(
        vm,
        i18n,
        truth.keyboards,
        truth.gamepads,
    ));
    // Canonical order owned by Rust; dynamic device sections (device.*)
    // keep build order after the known taxonomy.
    sections.sort_by_key(|section| {
        crate::settings_sections::SETTINGS_SECTION_IDS
            .iter()
            .position(|id| *id == section.id)
            .unwrap_or(usize::MAX)
    });
    sections
}

/// Play: what you touch every session. Machine-specific filters stay
/// per machine under Advanced (a filter value cannot be global).
fn build_play_section(
    vm: &SettingsViewModel,
    snapshot: &SettingsSnapshot,
    i18n: I18n,
) -> SectionWire {
    let audio = vm.audio.view.get();
    let orientation = snapshot.local.screen_orientation;
    SectionWire {
        id: "play".to_string(),
        label: crate::settings_sections::section_label("play", i18n),
        sub: String::new(),
        rows: vec![
            choice_row(
                "video.orientation",
                i18n.text(UiText::ScreenOrientation),
                i18n.text(UiText::DescOrientation),
                &[
                    (ScreenOrientation::Auto, i18n.text(UiText::AutoRotate)),
                    (ScreenOrientation::Portrait, i18n.text(UiText::Portrait)),
                    (ScreenOrientation::Landscape, i18n.text(UiText::Landscape)),
                ],
                &orientation,
            ),
            toggle_row(
                "audio.muted",
                i18n.text(UiText::Mute),
                String::new(),
                i18n,
                audio.muted,
            ),
            number_row(
                "audio.volume",
                i18n.text(UiText::MasterVolume),
                String::new(),
                i64::from(audio.volume_percent),
                1,
                0,
                100,
                "%",
            ),
            button_row("reset.play", i18n.text(UiText::ResetScreen), String::new()),
        ],
        visible: true,
    }
}

/// Saves: where save data lives (storage policy section rehoused).
fn build_saves_section(vm: &SettingsViewModel, i18n: I18n) -> SectionWire {
    let general = vm.general.view.get();
    let mut rows = vec![{
        let (choices, selected) = (
            general
                .storage_policy_choices
                .iter()
                .map(|choice| (choice.value, choice.label.clone()))
                .collect::<Vec<_>>(),
            general.storage_policy,
        );
        choice_row(
            "general.storage_policy",
            i18n.text(UiText::SaveStoragePolicy),
            i18n.text(UiText::DescStoragePolicy),
            &choices,
            &selected,
        )
    }];
    if general.show_storage_directory {
        rows.push(info_row(
            "general.storage_directory",
            i18n.text(UiText::SaveStorageDirectory),
            String::new(),
            general.storage_directory.clone(),
        ));
    }
    rows.push(button_row(
        "reset.saves",
        i18n.text(UiText::ResetScreen),
        String::new(),
    ));
    SectionWire {
        id: "saves".to_string(),
        label: crate::settings_sections::section_label("saves", i18n),
        sub: String::new(),
        rows,
        visible: true,
    }
}

/// App: language, about, global reset. The reset button confirms in
/// Kotlin first (`reset.all`), then restores every setting to default.
fn build_app_section(
    vm: &SettingsViewModel,
    snapshot: &SettingsSnapshot,
    i18n: I18n,
) -> SectionWire {
    let view = vm.general.view.get();
    let rows = vec![
        {
            let (choices, selected) = (
                view.language_choices
                    .iter()
                    .map(|choice| (choice.value, choice.label.clone()))
                    .collect::<Vec<_>>(),
                // Draft choice, not the view's concretized display copy:
                // with SystemDefault the view reads back the resolved
                // locale, which would highlight the wrong row.
                snapshot.shared.general.language,
            );
            choice_row(
                "general.language",
                i18n.text(UiText::Language),
                String::new(),
                &choices,
                &selected,
            )
        },
        group_row(
            "app.about",
            i18n.text(UiText::About),
            String::new(),
            "about",
        ),
        button_row("reset.all", i18n.text(UiText::ResetSettings), String::new()),
    ];
    SectionWire {
        id: "app".to_string(),
        label: crate::settings_sections::section_label("app", i18n),
        sub: String::new(),
        rows,
        visible: true,
    }
}

/// About: version and license readouts (info rows need no description:
/// the label is self-evident).
fn build_about_section(i18n: I18n) -> SectionWire {
    SectionWire {
        id: "about".to_string(),
        label: i18n.text(UiText::About),
        sub: i18n.text(UiText::SectionApp),
        rows: vec![
            info_row(
                "about.version",
                i18n.text(UiText::Version),
                String::new(),
                format!("nerust {}", env!("CARGO_PKG_VERSION")),
            ),
            info_row(
                "about.license",
                i18n.text(UiText::License),
                String::new(),
                "MPL-2.0".to_string(),
            ),
        ],
        visible: false,
    }
}

/// Advanced: rarely touched settings. VSync inline; sample rate/latency
/// behind the audio-detail screen; machine sections behind machine rows.
fn build_advanced_section(
    vm: &SettingsViewModel,
    snapshot: &SettingsSnapshot,
    i18n: I18n,
) -> SectionWire {
    let video = vm.video.view.get();
    let peripherals = &snapshot.local.cartridge_peripherals;
    let mut rows = vec![
        toggle_row(
            "video.vsync",
            i18n.text(UiText::Vsync),
            String::new(),
            i18n,
            video.vsync,
        ),
        group_row(
            "advanced.audio_detail",
            i18n.text(UiText::AudioDetail),
            i18n.text(UiText::DescAudioDetail),
            "audio.detail",
        ),
        toggle_row(
            "controls.cartridge_motion",
            i18n.text(UiText::CartridgeMotion),
            i18n.text(UiText::DescCartridgeMotion),
            i18n,
            peripherals.motion_enabled,
        ),
        toggle_row(
            "controls.cartridge_rumble",
            i18n.text(UiText::CartridgeRumble),
            i18n.text(UiText::DescCartridgeRumble),
            i18n,
            peripherals.rumble_enabled,
        ),
        number_row(
            "controls.cartridge_rumble_strength",
            i18n.text(UiText::RumbleStrength),
            String::new(),
            i64::from(peripherals.rumble_strength_percent),
            1,
            0,
            100,
            "%",
        ),
        choice_row(
            "controls.cartridge_rumble_target",
            i18n.text(UiText::RumbleTarget),
            String::new(),
            &[
                (RumbleTarget::Auto, i18n.text(UiText::Auto)),
                (RumbleTarget::Handset, i18n.text(UiText::Handset)),
                (RumbleTarget::Controller, i18n.text(UiText::Controller)),
            ],
            &peripherals.rumble_target,
        ),
    ];
    for system in vm.systems() {
        let sid = system.system_id().to_string();
        rows.push(group_row(
            &format!("advanced.{sid}"),
            system.display_name().to_string(),
            String::new(),
            &format!("system.{sid}"),
        ));
    }
    SectionWire {
        id: "advanced".to_string(),
        label: crate::settings_sections::section_label("advanced", i18n),
        sub: String::new(),
        rows,
        visible: true,
    }
}

/// Audio detail: sample rate and latency for crackle troubleshooting.
/// Reached from Advanced; everyday volume lives on Play.
fn build_audio_detail_section(vm: &SettingsViewModel, i18n: I18n) -> SectionWire {
    let view = vm.audio.view.get();
    SectionWire {
        id: "audio.detail".to_string(),
        label: i18n.text(UiText::AudioDetail),
        sub: i18n.text(UiText::SectionAdvanced),
        rows: vec![
            {
                let (choices, selected) = (
                    view.sample_rate_choices
                        .iter()
                        .map(|choice| (choice.value, choice.label.clone()))
                        .collect::<Vec<_>>(),
                    view.sample_rate,
                );
                choice_row(
                    "audio.sample_rate",
                    i18n.text(UiText::SampleRate),
                    i18n.text(UiText::DescSampleRate),
                    &choices,
                    &selected,
                )
            },
            number_row(
                "audio.latency",
                i18n.text(UiText::AudioLatency),
                i18n.text(UiText::DescLatency),
                i64::from(view.latency_ms),
                5,
                10,
                200,
                " ms",
            ),
            button_row(
                "reset.audio.detail",
                i18n.text(UiText::ResetScreen),
                String::new(),
            ),
        ],
        visible: false,
    }
}

fn build_notifications_section(vm: &SettingsViewModel, i18n: I18n) -> SectionWire {
    let view = vm.notifications.view.get();
    let colors: Vec<(NotificationColor, String)> = view
        .color_choices
        .iter()
        .map(|choice| (choice.value, choice.label.clone()))
        .collect();
    let positions: Vec<(NotificationPosition, String)> = view
        .position_choices
        .iter()
        .map(|choice| (choice.value, choice.label.clone()))
        .collect();
    SectionWire {
        id: "notifications".to_string(),
        label: crate::settings_sections::section_label("notifications", i18n),
        sub: String::new(),
        rows: vec![
            header_row(i18n.text(UiText::NotificationSectionDisplay)),
            toggle_row(
                "notifications.info_enabled",
                i18n.text(UiText::NotificationInfo),
                i18n.text(UiText::DescNotifInfo),
                i18n,
                view.info_enabled,
            ),
            number_row(
                "notifications.info_duration",
                i18n.text(UiText::NotificationDuration),
                String::new(),
                i64::from(view.info_duration_ms),
                500,
                1000,
                30000,
                " ms",
            ),
            choice_row(
                "notifications.info_color",
                i18n.text(UiText::NotificationInfo),
                String::new(),
                &colors,
                &view.info_color,
            ),
            toggle_row(
                "notifications.warning_enabled",
                i18n.text(UiText::NotificationWarning),
                i18n.text(UiText::DescNotifWarning),
                i18n,
                view.warning_enabled,
            ),
            number_row(
                "notifications.warning_duration",
                i18n.text(UiText::NotificationDuration),
                String::new(),
                i64::from(view.warning_duration_ms),
                500,
                1000,
                30000,
                " ms",
            ),
            choice_row(
                "notifications.warning_color",
                i18n.text(UiText::NotificationWarning),
                String::new(),
                &colors,
                &view.warning_color,
            ),
            toggle_row(
                "notifications.error_enabled",
                i18n.text(UiText::NotificationError),
                i18n.text(UiText::DescNotifError),
                i18n,
                view.error_enabled,
            ),
            toggle_row(
                "notifications.error_modal",
                i18n.text(UiText::NotificationErrorModal),
                i18n.text(UiText::DescNotifErrorModal),
                i18n,
                view.error_modal,
            ),
            choice_row(
                "notifications.error_color",
                i18n.text(UiText::NotificationError),
                String::new(),
                &colors,
                &view.error_color,
            ),
            choice_row(
                "notifications.position",
                i18n.text(UiText::NotificationPosition),
                String::new(),
                &positions,
                &view.position,
            ),
            choice_row(
                "notifications.area_mode",
                i18n.text(UiText::NotificationAreaMode),
                i18n.text(UiText::DescNotifArea),
                &view
                    .area_mode_choices
                    .iter()
                    .map(|choice| (choice.value, choice.label.clone()))
                    .collect::<Vec<_>>(),
                &view.area_mode,
            ),
            header_row(i18n.text(UiText::NotificationSectionStatus)),
            toggle_row(
                "status.enabled",
                i18n.text(UiText::NotificationStatus),
                String::new(),
                i18n,
                view.status_enabled,
            ),
            choice_row(
                "status.position",
                i18n.text(UiText::NotificationStatusPosition),
                String::new(),
                &positions,
                &view.status_position,
            ),
            {
                let mut row = choice_row(
                    "status.area_mode",
                    i18n.text(UiText::NotificationStatusAreaMode),
                    i18n.text(UiText::DescStatusArea),
                    &view
                        .area_mode_choices
                        .iter()
                        .map(|choice| (choice.value, choice.label.clone()))
                        .collect::<Vec<_>>(),
                    &view.status_area_mode,
                );
                // Same-edge rule: the status area follows the notification
                // area when both share an edge, so the row hides then.
                row.visible = view.position.is_top() != view.status_position.is_top();
                row
            },
            button_row(
                "reset.notifications",
                i18n.text(UiText::ResetScreen),
                String::new(),
            ),
        ],
        visible: true,
    }
}

/// Controllers: player ports first, then the touch overlay (a global screen
/// UI, never a port device), then global shortcuts (port-independent and
/// always active). The overlay detail holds the size/position/transparency
/// sliders; haptics stays here so there is exactly one place to toggle it.
/// Cartridge pak settings live under Advanced (set-and-forget attic).
/// Controllers quick screen: player port rows plus overlay/shortcut links.
fn controllers_quick_section(
    snapshot: &SettingsSnapshot,
    i18n: I18n,
    keyboards: &[KeyboardInfo],
    gamepads: &[GamepadInfo],
    touch_capable: Option<bool>,
) -> SectionWire {
    let overlay = &snapshot.local.touch_overlay;
    let mut quick_rows = Vec::new();
    for (index, port) in PLAYER_PORTS.iter().enumerate() {
        let player_id = player_id(index);
        let names = port_member_names(port, snapshot, keyboards, gamepads, i18n);
        quick_rows.push(group_row(
            &format!("controllers.{player_id}"),
            player_label(index, i18n),
            i18n.port_count_line(names.len()),
            &player_id,
        ));
        let last = quick_rows.last_mut().expect("player row pushed");
        last.text = i18n.port_summary(&names);
    }
    let mut visibility_row = choice_row(
        "controls.overlay_visibility",
        i18n.text(UiText::TouchOverlay),
        i18n.text(UiText::DescOverlayVisibility),
        &[
            (TouchOverlayVisibility::Always, i18n.text(UiText::Always)),
            (TouchOverlayVisibility::Auto, i18n.text(UiText::Auto)),
            (TouchOverlayVisibility::Hidden, i18n.text(UiText::Hidden)),
        ],
        &overlay.visibility,
    );
    if touch_capable == Some(false) {
        // Terminals without a touch screen can never show the overlay:
        // disable Always/Auto with the reason instead of pretending the
        // choice works and hiding at runtime. Order matches the literal
        // above (Always, Auto, Hidden).
        let reason = i18n.text(UiText::NoTouchScreen);
        for choice in visibility_row.choices.iter_mut().take(2) {
            choice.enabled = false;
            choice.reason = reason.clone();
        }
    }
    quick_rows.extend([
        header_row(i18n.text(UiText::TouchOverlay)),
        visibility_row,
        group_row(
            "controls.overlay_detail_link",
            i18n.text(UiText::OverlayDetail),
            i18n.text(UiText::DescOverlayDetail),
            "controls.overlay_detail",
        ),
        toggle_row(
            "controls.overlay_haptics",
            i18n.text(UiText::OverlayHaptics),
            String::new(),
            i18n,
            overlay.haptics,
        ),
        group_row(
            "controls.gamepads_link",
            i18n.text(UiText::Gamepads),
            i18n.text(UiText::DescDeadzone),
            "controls.gamepads",
        ),
        header_row(i18n.text(UiText::Shortcuts)),
        group_row(
            "controls.shortcuts_link",
            i18n.text(UiText::Shortcuts),
            i18n.text(UiText::DescShortcutsLink),
            "controls.shortcuts",
        ),
        button_row(
            "reset.controllers",
            i18n.text(UiText::ResetScreen),
            String::new(),
        ),
    ]);
    SectionWire {
        id: "controllers".to_string(),
        label: crate::settings_sections::section_label("controllers", i18n),
        sub: String::new(),
        rows: quick_rows,
        visible: true,
    }
}

/// Overlay detail screen: gamepad status plus opacity/scale/offset.
fn overlay_detail_section(
    overlay: &nerust_settings_core::model::local::TouchOverlaySettings,
    gamepads: &[GamepadInfo],
    i18n: I18n,
) -> SectionWire {
    SectionWire {
        id: "controls.overlay_detail".to_string(),
        label: i18n.text(UiText::TouchOverlay),
        sub: i18n.text(UiText::SectionControllers),
        rows: vec![
            info_row(
                "controls.auto_hide_status",
                i18n.text(UiText::ConnectedGamepads),
                String::new(),
                if gamepads.is_empty() {
                    i18n.text(UiText::NotAssigned)
                } else {
                    gamepads
                        .iter()
                        .map(|info| info.name.clone())
                        .collect::<Vec<_>>()
                        .join(" · ")
                },
            ),
            number_row(
                "controls.overlay_opacity",
                i18n.text(UiText::OverlayOpacity),
                String::new(),
                i64::from(overlay.opacity_percent),
                1,
                0,
                100,
                "%",
            ),
            number_row(
                "controls.overlay_scale",
                i18n.text(UiText::OverlayScale),
                String::new(),
                i64::from(overlay.scale_percent),
                5,
                50,
                150,
                "%",
            ),
            number_row(
                "controls.overlay_vertical_offset",
                i18n.text(UiText::OverlayVerticalOffset),
                String::new(),
                i64::from(overlay.vertical_offset_percent),
                1,
                -30,
                30,
                "%",
            ),
            button_row(
                "reset.controls.overlay_detail",
                i18n.text(UiText::ResetScreen),
                String::new(),
            ),
        ],
        visible: false,
    }
}

/// Gamepads screen: stick deadzone plus one port row per connected pad.
/// The list is variable: each connected pad names its own row, so Pad 1
/// is always the first-seen pad while it stays connected. Ports are
/// player slots, not devices: the connection-order table (not device
/// ids) is what persists. Unconnected pads hide; their assignment stays
/// in the table and reappears on reconnect.
fn gamepads_section(
    snapshot: &SettingsSnapshot,
    i18n: I18n,
    gamepads: &[GamepadInfo],
) -> SectionWire {
    let gamepad = &snapshot.shared.input.gamepad;
    let port_choices = [
        (None, i18n.text(UiText::Off)),
        (Some(0), i18n.text(UiText::PlayerOne)),
        (Some(1), i18n.text(UiText::PlayerTwo)),
    ];
    let mut rows = vec![number_row(
        "controls.gamepads.deadzone",
        i18n.text(UiText::Deadzone),
        i18n.text(UiText::DescDeadzone),
        i64::from(gamepad.deadzone_percent),
        5,
        0,
        30,
        "%",
    )];
    if gamepads.is_empty() {
        rows.push(info_row(
            "controls.gamepads.none",
            i18n.text(UiText::NotAssigned),
            i18n.text(UiText::DescNoGamepads),
            String::new(),
        ));
    }
    rows.extend(gamepads.iter().enumerate().flat_map(|(index, info)| {
        let pad_tag = nerust_settings_core::i18n::pad_label(i18n.language(), index);
        let port_row = choice_row(
            &format!("controls.gamepads.pad{}", index + 1),
            info.name.clone(),
            pad_tag,
            &port_choices,
            &gamepad.pad_ports.get(index).and_then(|pad| pad.slot),
        );
        let trigger_row = toggle_row(
            &format!("controls.gamepads.triggers{}", index + 1),
            nerust_settings_core::i18n::pad_triggers_label(i18n.language(), index),
            info.name.clone(),
            i18n,
            gamepad
                .pad_ports
                .get(index)
                .is_some_and(|pad| pad.triggers_as_shoulders),
        );
        [port_row, trigger_row]
    }));
    rows.push(button_row(
        "reset.controls.gamepads",
        i18n.text(UiText::ResetScreen),
        String::new(),
    ));
    SectionWire {
        id: "controls.gamepads".to_string(),
        label: i18n.text(UiText::Gamepads),
        sub: i18n.text(UiText::SectionControllers),
        rows,
        visible: false,
    }
}

/// Global shortcuts screen: one capture row per binding plus reset.
/// Fullscreen and debugger are desktop-only (window chrome the phone has
/// no equivalent for): their rows never reach Android, though the shared
/// bindings persist for desktop. Capture takes any mapped hardware key,
/// keyboard or pad; with neither device connected the rows disable with
/// a reason instead of opening a dead prompt.
fn shortcuts_section(
    vm: &SettingsViewModel,
    i18n: I18n,
    keyboards: &[KeyboardInfo],
    gamepads: &[GamepadInfo],
) -> SectionWire {
    let shortcuts_view = vm.shortcuts.view.get();
    let open = capture_open(keyboards, gamepads);
    let reason = i18n.text(UiText::InputDeviceRequired);
    // Shortcut targets encode a system only as a decoding hint; every
    // global shortcut decodes identically, so the first system suffices.
    let shortcut_system = vm
        .systems()
        .first()
        .map(|system| system.system_id().to_string())
        .unwrap_or_default();
    SectionWire {
        id: "controls.shortcuts".to_string(),
        label: i18n.text(UiText::Shortcuts),
        sub: i18n.text(UiText::SectionControllers),
        rows: shortcuts_view
            .rows
            .iter()
            .filter(|binding| {
                !matches!(
                    binding.target,
                    CaptureTarget::Shortcut(
                        ShortcutAction::ToggleFullscreen | ShortcutAction::OpenDebugger
                    )
                )
            })
            .map(|binding| {
                capture_row(
                    &format!("controls.{}", binding_key(&binding.target)),
                    binding.label.clone(),
                    String::new(),
                    binding_value_text(binding),
                    &encode_target(&binding.target, &shortcut_system),
                    open,
                    &reason,
                )
            })
            .chain(std::iter::once(button_row(
                "reset.controls.shortcuts",
                i18n.text(UiText::ResetScreen),
                String::new(),
            )))
            .collect(),
        visible: false,
    }
}

fn build_controllers_sections(
    vm: &SettingsViewModel,
    snapshot: &SettingsSnapshot,
    i18n: I18n,
    keyboards: &[KeyboardInfo],
    touch_capable: Option<bool>,
    gamepads: &[GamepadInfo],
) -> Vec<SectionWire> {
    vec![
        controllers_quick_section(snapshot, i18n, keyboards, gamepads, touch_capable),
        overlay_detail_section(&snapshot.local.touch_overlay, gamepads, i18n),
        shortcuts_section(vm, i18n, keyboards, gamepads),
        gamepads_section(snapshot, i18n, gamepads),
    ]
}

/// One section per machine: settings fields plus a navigation row into the
/// machine's controller screen (`input.<system>`). Keyboard bindings live in
/// `keys.*`, one drill-down from each device detail screen.
fn build_system_sections(vm: &SettingsViewModel, i18n: I18n) -> Vec<SectionWire> {
    vm.systems()
        .iter()
        .map(|system| {
            let view = system.view.get();
            let mut rows: Vec<RowWire> = view
                .fields
                .iter()
                .map(|field| {
                    choice_row(
                        &format!("system.{}.{}", view.system_id, field.id.as_str()),
                        field.label.clone(),
                        String::new(),
                        &field
                            .choices
                            .iter()
                            .map(|choice| (choice.value.as_str().to_string(), choice.label.clone()))
                            .collect::<Vec<_>>(),
                        &field.selected.as_str().to_string(),
                    )
                })
                .collect();
            rows.push(group_row(
                &format!("system.{}.controllers", view.system_id),
                i18n.text(UiText::Controller),
                String::new(),
                &format!("input.{}", view.system_id),
            ));
            rows.push(button_row(
                &format!("reset.system.{}", view.system_id),
                i18n.text(UiText::ResetScreen),
                String::new(),
            ));
            SectionWire {
                id: format!("system.{}", view.system_id),
                label: view.label.clone(),
                sub: i18n.text(UiText::SectionAdvanced),
                rows,
                visible: false,
            }
        })
        .collect()
}

/// One controller screen per machine: the emulated controller type per
/// slot (IA "controller type is per machine port"). Hidden from the
/// category list; reached from the player type screens and from the
/// machine screen under Advanced. Keyboard bindings moved to `keys.*`.
fn build_input_sections(vm: &SettingsViewModel, i18n: I18n) -> Vec<SectionWire> {
    vm.inputs()
        .iter()
        .map(|input| {
            let view = input.view.get();
            let mut rows = slot_rows(&view, i18n);
            rows.push(button_row(
                &format!("reset.input.{}", view.system_id),
                i18n.text(UiText::ResetScreen),
                String::new(),
            ));
            SectionWire {
                id: format!("input.{}", view.system_id),
                label: view.label.clone(),
                sub: i18n.text(UiText::ControllerType),
                rows,
                visible: false,
            }
        })
        .collect()
}

/// Slot assignment rows shared by the machine controller screens.
fn slot_rows(view: &nerust_gui_viewmodel::settings::dto::InputTabView, i18n: I18n) -> Vec<RowWire> {
    view.slots
        .iter()
        .map(|slot| {
            let mut row = choice_row(
                &format!("system.{}.slot.{}", view.system_id, slot.slot_id.as_str()),
                slot.label.clone(),
                i18n.slot_description(),
                &slot
                    .choices
                    .iter()
                    .map(|choice| {
                        (
                            serde_json::to_value(&choice.value).unwrap_or(serde_json::Value::Null),
                            choice.label.clone(),
                        )
                    })
                    .collect::<Vec<_>>(),
                &serde_json::to_value(&slot.selected_profile_id).unwrap_or(serde_json::Value::Null),
            );
            // A multi-port profile (NES famicom set) already covers this
            // slot from another one: nothing but "not assigned" is
            // pickable here until that slot releases it. The desktop
            // switch disables the whole row; the wire has no row switch,
            // so every concrete choice carries the owner instead.
            // Anything else stays enabled: the same profile may drive two
            // slots at once (NES standard pad on P1 and P2), and the
            // editor clears obsoleted mates on pick — there is no
            // taken-by-id suppression (it wrongly blocked that sharing).
            let covering_owner = slot.occupied_by_other_slot.then(|| {
                view.slots
                    .iter()
                    .find(|other| {
                        other.slot_id != slot.slot_id && other.selected_profile_id.is_some()
                    })
                    .map(|other| other.label.clone())
                    .unwrap_or_default()
            });
            for (wire, choice) in row.choices.iter_mut().zip(slot.choices.iter()) {
                let covered = covering_owner.as_ref().filter(|_| choice.value.is_some());
                wire.enabled = covered.is_none();
                wire.reason = covered
                    .map(|owner| i18n.slot_in_use(owner.as_str()))
                    .unwrap_or_default();
            }
            row
        })
        .collect()
}

/// Player screens (one per global port): the Devices row opens the device
/// picker, the Controller-type row opens the per-machine type list.
/// Summaries read from the port device set plus pad-port assignments,
/// never from machine sections.
fn build_player_sections(
    vm: &SettingsViewModel,
    snapshot: &SettingsSnapshot,
    i18n: I18n,
    keyboards: &[KeyboardInfo],
    gamepads: &[GamepadInfo],
) -> Vec<SectionWire> {
    let mut sections = Vec::new();
    for (index, port) in PLAYER_PORTS.iter().enumerate() {
        let player_id = player_id(index);
        let devices_id = devices_id(port);
        let types_id = types_id(index);
        let label = player_label(index, i18n);
        let names = port_member_names(port, snapshot, keyboards, gamepads, i18n);
        sections.push(SectionWire {
            id: player_id.clone(),
            label: label.clone(),
            sub: i18n.text(UiText::SectionControllers),
            rows: vec![
                {
                    let mut row = group_row(
                        &format!("{player_id}.devices"),
                        i18n.text(UiText::Devices),
                        String::new(),
                        &devices_id,
                    );
                    row.text = i18n.port_summary(&names);
                    row
                },
                group_row(
                    &format!("{player_id}.type"),
                    i18n.text(UiText::ControllerType),
                    String::new(),
                    &types_id,
                ),
            ],
            visible: false,
        });
        // Controller-type list: one group row per machine that actually
        // has a slot on this port (NES-only for player 2). Slot ids must
        // end in player1/player2; fail loudly in tests when a factory
        // diverges instead of silently dropping its slots.
        debug_assert!(
            {
                let mut ok = true;
                for input in vm.inputs() {
                    for slot in &input.view.get().slots {
                        ok &= slot_port_index(slot.slot_id.as_str()).is_some();
                    }
                }
                ok
            },
            "slot ids must end with player1/player2"
        );
        let type_rows: Vec<RowWire> = vm
            .inputs()
            .iter()
            .filter(|input| {
                input
                    .view
                    .get()
                    .slots
                    .iter()
                    .any(|slot| slot_port_index(slot.slot_id.as_str()) == Some(index))
            })
            .map(|input| {
                let view = input.view.get();
                group_row(
                    &format!("{types_id}.{}", view.system_id),
                    view.label.clone(),
                    type_profile_summary(&view, index, i18n),
                    &format!("input.{}", view.system_id),
                )
            })
            .collect();
        sections.push(SectionWire {
            id: types_id.clone(),
            label: i18n.text(UiText::ControllerType),
            sub: label.clone(),
            rows: type_rows,
            visible: false,
        });
        sections.push(build_devices_section(
            port,
            &devices_id,
            &label,
            snapshot,
            keyboards,
            gamepads,
            i18n,
        ));
    }
    sections.extend(build_device_sections(vm, keyboards, i18n));
    sections
}

/// Slot id suffix rule: every factory names player slots
/// `<system>.attachment.playerN`. Player 2 exists on NES only.
fn slot_port_index(slot_id: &str) -> Option<usize> {
    if slot_id.ends_with("player2") {
        Some(1)
    } else if slot_id.ends_with("player1") {
        Some(0)
    } else {
        None
    }
}

/// Current controller-type summary for one machine on one port: the
/// selected profile label, or Not assigned when the slot is empty.
fn type_profile_summary(
    view: &nerust_gui_viewmodel::settings::dto::InputTabView,
    port_index: usize,
    i18n: I18n,
) -> String {
    view.slots
        .iter()
        .find(|slot| slot_port_index(slot.slot_id.as_str()) == Some(port_index))
        .and_then(|slot| {
            slot.selected_profile_id.as_ref().and_then(|selected| {
                slot.choices.iter().find_map(|choice| {
                    (choice.value.as_deref() == Some(selected.as_str()))
                        .then(|| choice.label.clone())
                })
            })
        })
        .unwrap_or_else(|| i18n.text(UiText::NotAssigned))
}

/// Display names for a port in set order: keyboards from the port device
/// set plus gamepads from the pad-port table. Unknown members fall back
/// to a generic label so a stale id never renders blank. Gamepad names
/// resolve by connection-order index (Pad N = Nth connected pad); an
/// assigned but disconnected pad keeps its `Pad N` label so the
/// assignment stays visible.
fn port_member_names(
    port: &str,
    snapshot: &SettingsSnapshot,
    keyboards: &[KeyboardInfo],
    gamepads: &[GamepadInfo],
    i18n: I18n,
) -> Vec<String> {
    // Effective members only: rows render solely while the terminal
    // enumerates them, so every screen shares one enumeration truth.
    // Otherwise a reconnectable USB keyboard would read as assigned on
    // the player screens while the device list reports none connected.
    // Unplugged USB descriptors stay assigned (reconnect restores them).
    let builtin_present = keyboards
        .iter()
        .any(|info| info.member == BUILTIN_KEYBOARD_MEMBER);
    let mut names: Vec<String> = snapshot
        .app_state
        .port_devices
        .get(port)
        .map(|members| {
            members
                .iter()
                .filter(|member| {
                    if member.as_str() == BUILTIN_KEYBOARD_MEMBER {
                        builtin_present
                    } else if member_is_shareable(member) {
                        keyboards.iter().any(|info| info.member == **member)
                    } else {
                        true
                    }
                })
                .map(|member| keyboard_member_name(member, keyboards, i18n))
                .collect()
        })
        .unwrap_or_default();
    let port_index = PLAYER_PORTS.iter().position(|known| *known == port);
    if let Some(port_index) = port_index {
        for (pad_index, pad) in snapshot.shared.input.gamepad.pad_ports.iter().enumerate() {
            if pad.slot == Some(port_index) {
                if let Some(info) = gamepads.get(pad_index) {
                    names.push(info.name.clone());
                }
            }
        }
    }
    names
}

/// Drop the built-in membership when fresh enumeration reports no
/// built-in keyboard: a capability flag, not a known device, so there is
/// nothing to restore on reconnect. USB descriptors are never pruned
/// here — unplug/replug cycles keep their assignment.
pub(super) fn prune_missing_builtin(snapshot: &mut SettingsSnapshot, keyboards: &[KeyboardInfo]) {
    // No evidence, no pruning: an empty scan (transient enumeration race)
    // must never wipe persisted membership. A non-empty scan missing the
    // built-in row is positive absence and prunes.
    if keyboards.is_empty() {
        return;
    }
    if keyboards
        .iter()
        .any(|info| info.member == BUILTIN_KEYBOARD_MEMBER)
    {
        return;
    }
    for members in snapshot.app_state.port_devices.values_mut() {
        members.retain(|member| member != BUILTIN_KEYBOARD_MEMBER);
    }
}

fn keyboard_member_name(member: &str, keyboards: &[KeyboardInfo], i18n: I18n) -> String {
    if member == BUILTIN_KEYBOARD_MEMBER {
        i18n.text(UiText::BuiltInKeyboard)
    } else if let Some(info) = keyboards.iter().find(|info| info.member == member) {
        if info.name.trim().is_empty() {
            // OS gave no name: fall back instead of rendering blank.
            i18n.text(UiText::ExternalKeyboard)
        } else {
            info.name.clone()
        }
    } else if member_is_shareable(member) {
        i18n.text(UiText::ExternalKeyboard)
    } else if member_is_gamepad(member) {
        i18n.text(UiText::GamepadDevice)
    } else {
        member.to_string()
    }
}

/// Device picker: the dynamic keyboard enumeration (built-in row when
/// present, one row per external descriptor, BT/USB agnostic, virtual
/// devices excluded — classified Kotlin-side). Row tap toggles port
/// membership; Details opens the device screen. Gamepads are
/// connection-order pads assigned under Controls > Gamepads, so this
/// screen lists the connected pads with their player assignment plus a
/// link there instead of port toggles (toggling here would edit a map
/// the session never reads).
fn build_devices_section(
    port: &str,
    devices_id: &str,
    player: &str,
    snapshot: &SettingsSnapshot,
    keyboards: &[KeyboardInfo],
    gamepads: &[GamepadInfo],
    i18n: I18n,
) -> SectionWire {
    let members = snapshot.app_state.port_devices.get(port);
    let is_member = |member: &str| {
        members
            .map(|set| set.iter().any(|entry| entry == member))
            .unwrap_or(false)
    };
    let mut rows = vec![header_row(i18n.text(UiText::Keyboard))];
    if keyboards.is_empty() {
        rows.push(info_row(
            &format!("{devices_id}.none"),
            i18n.text(UiText::NoKeyboards),
            i18n.text(UiText::DescNoKeyboards),
            String::new(),
        ));
    }
    for info in keyboards {
        let mut row = toggle_row(
            &format!("devices.{port}.{}", info.member),
            keyboard_member_name(&info.member, keyboards, i18n),
            if info.builtin {
                i18n.text(UiText::AlwaysAvailable)
            } else {
                i18n.text(UiText::Connected)
            },
            i18n,
            is_member(&info.member),
        );
        // The opaque member id rides the target; the port comes from the key.
        row.target = info.member.clone();
        row.detail_target = format!("device.{}", info.member);
        rows.push(row);
    }
    rows.push(header_row(i18n.text(UiText::Gamepads)));
    if gamepads.is_empty() {
        rows.push(info_row(
            &format!("{devices_id}.pads.none"),
            i18n.text(UiText::NotAssigned),
            String::new(),
            String::new(),
        ));
    }
    let port_index = PLAYER_PORTS.iter().position(|known| *known == port);
    for (pad_index, info) in gamepads.iter().enumerate() {
        let assigned = port_index.is_some_and(|port_index| {
            snapshot
                .shared
                .input
                .gamepad
                .pad_ports
                .get(pad_index)
                .is_some_and(|pad| pad.slot == Some(port_index))
        });
        let status = if assigned {
            player.to_string()
        } else {
            snapshot
                .shared
                .input
                .gamepad
                .pad_ports
                .get(pad_index)
                .and_then(|pad| pad.slot)
                .and_then(|slot| PLAYER_PORTS.get(slot))
                .map(|other| {
                    player_label(
                        PLAYER_PORTS.iter().position(|known| known == other).unwrap_or(0),
                        i18n,
                    )
                })
                .unwrap_or_else(|| i18n.text(UiText::NotAssigned))
        };
        rows.push(info_row(
            &format!("{devices_id}.{}", info.member),
            info.name.clone(),
            String::new(),
            status,
        ));
    }
    rows.push(group_row(
        &format!("{devices_id}.gamepads"),
        i18n.text(UiText::Gamepads),
        String::new(),
        "controls.gamepads",
    ));
    rows.push(button_row(
        &format!("reset.{devices_id}"),
        i18n.text(UiText::ResetScreen),
        String::new(),
    ));
    SectionWire {
        id: devices_id.to_string(),
        label: i18n.text(UiText::Devices),
        sub: player.to_string(),
        rows,
        visible: false,
    }
}

/// One device screen per enumerated keyboard: connection/capability
/// readouts (labels are self-evident, so no descriptions) plus the
/// per-system key-mapping tables (shared single table, not per device).
fn build_device_sections(
    vm: &SettingsViewModel,
    keyboards: &[KeyboardInfo],
    i18n: I18n,
) -> Vec<SectionWire> {
    keyboards
        .iter()
        .map(|info| {
            let mut rows = vec![
                info_row(
                    &format!("device.{}.connection", info.member),
                    i18n.text(UiText::Connection),
                    String::new(),
                    if info.builtin {
                        i18n.text(UiText::AlwaysAvailable)
                    } else {
                        i18n.text(UiText::Connected)
                    },
                ),
                info_row(
                    &format!("device.{}.capabilities", info.member),
                    i18n.text(UiText::Capabilities),
                    String::new(),
                    i18n.text(UiText::SharesAcrossPlayers),
                ),
                header_row(i18n.text(UiText::KeyMapping)),
            ];
            for system in vm.systems() {
                let sid = system.system_id().to_string();
                rows.push(group_row(
                    &format!("device.{}.keys.{sid}", info.member),
                    system.display_name().to_string(),
                    String::new(),
                    &format!("keys.{sid}"),
                ));
            }
            SectionWire {
                id: format!("device.{}", info.member),
                label: if info.builtin {
                    i18n.text(UiText::BuiltInKeyboard)
                } else if info.name.trim().is_empty() {
                    i18n.text(UiText::ExternalKeyboard)
                } else {
                    info.name.clone()
                },
                sub: i18n.text(UiText::Devices),
                rows,
                visible: false,
            }
        })
        .collect()
}
/// One key-mapping screen per machine: the shared single keyboard table
/// (per-device split is future work). Reached from each keyboard device
/// detail screen; the controller screens hold slots only. Capture takes
/// any mapped hardware key; with no input device the rows disable.
fn build_keys_sections(
    vm: &SettingsViewModel,
    i18n: I18n,
    keyboards: &[KeyboardInfo],
    gamepads: &[GamepadInfo],
) -> Vec<SectionWire> {
    let open = capture_open(keyboards, gamepads);
    let reason = i18n.text(UiText::InputDeviceRequired);
    vm.inputs()
        .iter()
        .map(|input| {
            let view = input.view.get();
            let mut rows: Vec<RowWire> = Vec::new();
            for section in &view.sections {
                rows.push(header_row(section.label.clone()));
                rows.extend(section.rows.iter().map(|binding| {
                    capture_row(
                        &format!(
                            "keys.{}.binding.{}",
                            view.system_id,
                            binding_key(&binding.target)
                        ),
                        binding.label.clone(),
                        String::new(),
                        binding_value_text(binding),
                        &encode_target(&binding.target, &view.system_id.to_string()),
                        open,
                        &reason,
                    )
                }));
            }
            rows.push(button_row(
                &format!("reset.keys.{}", view.system_id),
                i18n.text(UiText::ResetScreen),
                String::new(),
            ));
            SectionWire {
                id: format!("keys.{}", view.system_id),
                label: view.label.clone(),
                sub: i18n.text(UiText::KeyMapping),
                rows,
                visible: false,
            }
        })
        .collect()
}

fn binding_key(target: &CaptureTarget) -> String {
    match target {
        CaptureTarget::Binding {
            attachment,
            control,
            ..
        } => format!("{attachment}.{control}"),
        CaptureTarget::Shortcut(action) => format!("shortcut.{action:?}"),
    }
}

fn binding_value_text(binding: &nerust_gui_viewmodel::settings::dto::BindingRowView) -> String {
    use nerust_gui_viewmodel::settings::dto::BindingValueView;
    match &binding.value {
        BindingValueView::Unbound(label)
        | BindingValueView::Bound(label)
        | BindingValueView::Capturing(label) => label.clone(),
    }
}

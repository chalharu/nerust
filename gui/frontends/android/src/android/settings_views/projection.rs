//! Android settings projection: view-model DTOs rendered into wire sections.
//!
//! Every builder here is pure: it reads the view model, the draft snapshot,
//! and the vault device truth, and returns wire rows. No mutation.

use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_gui_viewmodel::settings::SettingsViewModel;
use nerust_settings_core::{
    editor::CaptureTarget,
    i18n::UiText,
    model::local::{
        NotificationColor, NotificationPosition, RumbleTarget, ScreenOrientation,
        TouchOverlayVisibility,
    },
};

use super::devices::{build_keys_sections, build_player_sections, port_member_names};
use super::pads::build_pad_sections;
use super::wire::{
    CaptureSlot, CaptureWire, GamepadInfo, I18n, KeyboardInfo, PLAYER_PORTS, Projection, RowWire,
    SectionWire, ViewPayload, VmDeps, binding_key, build_vm, button_row, capture_row, choice_row,
    draft_issues, encode_target, group_row, header_row, info_row, number_row, player_id,
    player_label, target_system, toggle_row,
};
use crate::bridge_contract::SETTINGS_SCHEMA_VERSION;

/// Assemble the wire payload from an already-built view model: sections,
/// live issues from a non-fatal `finish` probe, and the pending capture
/// state. The single projection site: every command arm funnels here.
pub(super) fn project_payload(
    vm: &SettingsViewModel,
    snapshot: &SettingsSnapshot,
    capture: &CaptureSlot,
    truth: Projection,
) -> ViewPayload {
    ViewPayload {
        schema_version: SETTINGS_SCHEMA_VERSION,
        sections: build_sections(vm, snapshot, truth),
        issues: draft_issues(vm, truth.i18n),
        capture: capture.as_ref().map(|(target, prompt)| CaptureWire {
            target: encode_target(target, &target_system(target)),
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
    Ok(project_payload(&vm, snapshot, capture, truth))
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
    // keep build order after the known taxonomy. Rank map hoisted out of
    // the comparator: position-in-sort per comparison is O(n^2).
    let rank: std::collections::HashMap<&str, usize> =
        crate::settings_sections::SETTINGS_SECTION_IDS
            .iter()
            .enumerate()
            .map(|(index, id)| (*id, index))
            .collect();
    sections.sort_by_key(|section| rank.get(section.id.as_str()).copied().unwrap_or(usize::MAX));
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
) -> SectionWire {
    let mut quick_rows = Vec::new();
    for (index, port) in PLAYER_PORTS.iter().enumerate() {
        let player_id = player_id(index);
        let names = port_member_names(port, snapshot, keyboards, gamepads, i18n);
        quick_rows.push(group_row(
            &format!("controllers.{player_id}"),
            player_label(index, i18n),
            nerust_gui_viewmodel::settings::labels::port_count_line(i18n, names.len()),
            &player_id,
        ));
        let last = quick_rows.last_mut().expect("player row pushed");
        last.text = nerust_gui_viewmodel::settings::labels::port_summary(i18n, &names);
    }
    // Connected pads link straight to their own screen (deadzone,
    // hat switch, button mapping).
    if !gamepads.is_empty() {
        quick_rows.push(header_row(i18n.text(UiText::Gamepads)));
    }
    // Past the persisted table a pad parks with no settings of its
    // own: no link, no screen.
    for (index, info) in gamepads
        .iter()
        .take(nerust_input_traits::MAX_PADS)
        .enumerate()
    {
        quick_rows.push(group_row(
            &format!("controls.pad{}.link", index + 1),
            info.name.clone(),
            i18n.text(UiText::DescGamepadGear),
            &format!("pad.{}", index + 1),
        ));
    }
    // Pure navigation hub like player/about: every editable row lives on
    // a detail screen with its own reset, so this screen carries none.
    // Overlay display rows (visibility, haptics, sliders) all live on
    // `controls.overlay_detail`, one screen per feature.
    quick_rows.extend([
        header_row(i18n.text(UiText::TouchOverlay)),
        group_row(
            "controls.overlay_detail_link",
            i18n.text(UiText::OverlayDetail),
            i18n.text(UiText::DescOverlayDetail),
            "controls.overlay_detail",
        ),
        header_row(i18n.text(UiText::Shortcuts)),
        group_row(
            "controls.shortcuts_link",
            i18n.text(UiText::Shortcuts),
            i18n.text(UiText::DescShortcutsLink),
            "controls.shortcuts",
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

/// Overlay detail screen: every overlay setting (visibility, haptics,
/// opacity/scale/offset) plus the gamepad status readout. One screen per
/// feature, so the screen reset below covers the whole screen.
fn overlay_detail_section(
    overlay: &nerust_settings_core::model::local::TouchOverlaySettings,
    gamepads: &[GamepadInfo],
    touch_capable: Option<bool>,
    i18n: I18n,
) -> SectionWire {
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
                    i18n.text(UiText::NoValue)
                } else {
                    gamepads
                        .iter()
                        .map(|info| info.name.clone())
                        .collect::<Vec<_>>()
                        .join(", ")
                },
            ),
            visibility_row,
            toggle_row(
                "controls.overlay_haptics",
                i18n.text(UiText::OverlayHaptics),
                String::new(),
                i18n,
                overlay.haptics,
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

/// Global shortcuts screen: one device block per CONNECTED input, each
/// with one capture row per binding. The keyboard block shows only while
/// a keyboard is connected (keyboards are hot-pluggable like pads now);
/// every connected pad gets its own `Gamepad (name)` block showing only
/// its pad column. Capture accepts either input from any row and lands
/// it in the matching column; Clear on a row clears only that row's
/// column. With neither device connected, an empty-state row explains
/// itself instead of opening a dead prompt. Fullscreen and debugger are
/// desktop-only (window chrome the phone has no equivalent for): their
/// rows never reach Android, though the shared bindings persist.
fn shortcuts_section(
    vm: &SettingsViewModel,
    snapshot: &SettingsSnapshot,
    i18n: I18n,
    keyboards: &[KeyboardInfo],
    gamepads: &[GamepadInfo],
) -> SectionWire {
    let reason = i18n.text(UiText::InputDeviceRequired);
    // Shortcut targets encode no system (the encoder drops it for the
    // `shortcut` kind and the decoder never reads it); bindings carry
    // their own. No invented hint.
    let actions: Vec<_> = vm
        .shortcuts
        .view
        .get()
        .rows
        .iter()
        .filter(|binding| {
            // The phone has no window chrome (fullscreen, debugger): rows
            // needing it never reach Android, though shared bindings persist.
            binding.target.action().is_none_or(|action| {
                !matches!(
                    action,
                    nerust_settings_core::model::input::ShortcutAction::ToggleFullscreen
                        | nerust_settings_core::model::input::ShortcutAction::OpenDebugger
                )
            })
        })
        .map(|binding| {
            (
                binding.label.clone(),
                encode_target(&binding.target, ""),
                binding.target.clone(),
            )
        })
        .collect();
    let mut rows = Vec::new();
    if !keyboards.is_empty() {
        rows.push(header_row(i18n.text(UiText::Keyboard)));
        for (label, target, capture) in &actions {
            rows.push(capture_row(
                &format!("controls.{}.kbd", binding_key(capture)),
                label.clone(),
                String::new(),
                shortcut_binding_text(snapshot, capture, i18n, false),
                target,
                true,
                &reason,
            ));
        }
    }
    // One shared pad block: the pad column is global (one button per
    // action across all pads), so per-pad blocks would show N copies of
    // the same value. The block lists every connected pad by name; capture
    // from any row accepts any pad's button.
    if !gamepads.is_empty() {
        let names: Vec<_> = gamepads
            .iter()
            .take(nerust_input_traits::MAX_PADS)
            .map(|info| info.name.clone())
            .collect();
        rows.push(header_row(format!(
            "{} ({})",
            i18n.text(UiText::GamepadDevice),
            names.join(", ")
        )));
        for (label, target, capture) in &actions {
            rows.push(capture_row(
                &format!("controls.{}.pad", binding_key(capture)),
                label.clone(),
                String::new(),
                shortcut_binding_text(snapshot, capture, i18n, true),
                target,
                true,
                &reason,
            ));
        }
    }
    if rows.is_empty() {
        rows.push(info_row(
            "controls.shortcuts.none",
            i18n.text(UiText::NoInputDevices),
            i18n.text(UiText::DescNoInputDevices),
            String::new(),
        ));
    }
    rows.push(button_row(
        "reset.controls.shortcuts",
        i18n.text(UiText::ResetScreen),
        String::new(),
    ));
    SectionWire {
        id: "controls.shortcuts".to_string(),
        label: i18n.text(UiText::Shortcuts),
        sub: i18n.text(UiText::SectionControllers),
        rows,
        visible: false,
    }
}

/// One column of one shortcut binding for the per-device rows: the key
/// column or the pad column, Unbound when empty.
fn shortcut_binding_text(
    snapshot: &SettingsSnapshot,
    target: &CaptureTarget,
    i18n: I18n,
    pad: bool,
) -> String {
    let label = if pad {
        nerust_settings_core::editor::current_pad_binding(snapshot, target)
            .map(|button| nerust_gui_viewmodel::settings::labels::pad_button_label(i18n, button))
    } else {
        nerust_settings_core::editor::current_binding_key(snapshot, target)
            .map(|key| key.label().to_string())
    };
    label.unwrap_or_else(|| i18n.text(UiText::Unbound))
}

fn build_controllers_sections(
    vm: &SettingsViewModel,
    snapshot: &SettingsSnapshot,
    i18n: I18n,
    keyboards: &[KeyboardInfo],
    touch_capable: Option<bool>,
    gamepads: &[GamepadInfo],
) -> Vec<SectionWire> {
    let mut sections = vec![
        controllers_quick_section(snapshot, i18n, keyboards, gamepads),
        overlay_detail_section(&snapshot.local.touch_overlay, gamepads, touch_capable, i18n),
        shortcuts_section(vm, snapshot, i18n, keyboards, gamepads),
    ];
    sections.extend(build_pad_sections(vm, snapshot, i18n, gamepads));
    sections
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
            // Qualify with the system display name: every machine names its
            // slots "Player 1" (GBC/GBA have only one), so the bare label
            // repeats as the choice detail title across NES/GBC/GBA.
            // The detail screen shows the row label as its title, and the
            // subtitle only names the screen kind, never the machine.
            let label = format!("{} {}", view.label, slot.label);
            let mut row = choice_row(
                &format!("system.{}.slot.{}", view.system_id, slot.slot_id.as_str()),
                label,
                nerust_gui_viewmodel::settings::labels::slot_description(i18n),
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
                    .map(|owner| {
                        nerust_gui_viewmodel::settings::labels::slot_in_use(i18n, owner.as_str())
                    })
                    .unwrap_or_default();
            }
            row
        })
        .collect()
}

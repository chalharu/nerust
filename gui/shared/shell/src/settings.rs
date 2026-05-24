use crate::load::NesLoadOptions;
use nerust_contract_settings::{
    desktop::{DesktopSettings, SystemSettings},
    input::{
        BindingProfile, ControlBinding, HostInputSource, KeyboardKey, PersistedAttachmentId,
        PersistedControlId,
    },
    nes::{NesSettings, NesVideoFilter},
    shortcut::{ShortcutAction, ShortcutBinding},
};
use nerust_gui_runtime::settings::{DesktopSettingsManager, SettingsError};
use nerust_gui_session::commands::SessionCommand;
use nerust_input_nes::topology::{
    FAMICOM_P2_CONTROL_MICROPHONE, NES_ATTACHMENT_PLAYER_ONE, NES_ATTACHMENT_PLAYER_TWO,
    NES_CONTROL_A, NES_CONTROL_B, NES_CONTROL_DOWN, NES_CONTROL_LEFT, NES_CONTROL_RIGHT,
    NES_CONTROL_SELECT, NES_CONTROL_START, NES_CONTROL_UP,
};
use nerust_input_schema::{AttachmentId, DigitalControlId, DigitalInputEvent, SystemId};
use nerust_screen_buffer::screen_buffer::ScreenBuffer;
use nerust_screen_filter::FilterType;
use nerust_sound_openal::OpenAl;
use nerust_timer::CLOCK_RATE;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NesKeyboardBindingDescriptor {
    pub attachment: AttachmentId,
    pub attachment_label: &'static str,
    pub control: DigitalControlId,
    pub control_label: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShortcutDescriptor {
    pub action: ShortcutAction,
    pub label: &'static str,
}

pub const NES_KEYBOARD_BINDINGS: &[NesKeyboardBindingDescriptor] = &[
    NesKeyboardBindingDescriptor {
        attachment: NES_ATTACHMENT_PLAYER_ONE,
        attachment_label: "Player 1",
        control: NES_CONTROL_A,
        control_label: "A",
    },
    NesKeyboardBindingDescriptor {
        attachment: NES_ATTACHMENT_PLAYER_ONE,
        attachment_label: "Player 1",
        control: NES_CONTROL_B,
        control_label: "B",
    },
    NesKeyboardBindingDescriptor {
        attachment: NES_ATTACHMENT_PLAYER_ONE,
        attachment_label: "Player 1",
        control: NES_CONTROL_SELECT,
        control_label: "Select",
    },
    NesKeyboardBindingDescriptor {
        attachment: NES_ATTACHMENT_PLAYER_ONE,
        attachment_label: "Player 1",
        control: NES_CONTROL_START,
        control_label: "Start",
    },
    NesKeyboardBindingDescriptor {
        attachment: NES_ATTACHMENT_PLAYER_ONE,
        attachment_label: "Player 1",
        control: NES_CONTROL_UP,
        control_label: "Up",
    },
    NesKeyboardBindingDescriptor {
        attachment: NES_ATTACHMENT_PLAYER_ONE,
        attachment_label: "Player 1",
        control: NES_CONTROL_DOWN,
        control_label: "Down",
    },
    NesKeyboardBindingDescriptor {
        attachment: NES_ATTACHMENT_PLAYER_ONE,
        attachment_label: "Player 1",
        control: NES_CONTROL_LEFT,
        control_label: "Left",
    },
    NesKeyboardBindingDescriptor {
        attachment: NES_ATTACHMENT_PLAYER_ONE,
        attachment_label: "Player 1",
        control: NES_CONTROL_RIGHT,
        control_label: "Right",
    },
    NesKeyboardBindingDescriptor {
        attachment: NES_ATTACHMENT_PLAYER_TWO,
        attachment_label: "Player 2",
        control: NES_CONTROL_A,
        control_label: "A",
    },
    NesKeyboardBindingDescriptor {
        attachment: NES_ATTACHMENT_PLAYER_TWO,
        attachment_label: "Player 2",
        control: NES_CONTROL_B,
        control_label: "B",
    },
    NesKeyboardBindingDescriptor {
        attachment: NES_ATTACHMENT_PLAYER_TWO,
        attachment_label: "Player 2",
        control: NES_CONTROL_UP,
        control_label: "Up",
    },
    NesKeyboardBindingDescriptor {
        attachment: NES_ATTACHMENT_PLAYER_TWO,
        attachment_label: "Player 2",
        control: NES_CONTROL_DOWN,
        control_label: "Down",
    },
    NesKeyboardBindingDescriptor {
        attachment: NES_ATTACHMENT_PLAYER_TWO,
        attachment_label: "Player 2",
        control: NES_CONTROL_LEFT,
        control_label: "Left",
    },
    NesKeyboardBindingDescriptor {
        attachment: NES_ATTACHMENT_PLAYER_TWO,
        attachment_label: "Player 2",
        control: NES_CONTROL_RIGHT,
        control_label: "Right",
    },
    NesKeyboardBindingDescriptor {
        attachment: NES_ATTACHMENT_PLAYER_TWO,
        attachment_label: "Player 2",
        control: FAMICOM_P2_CONTROL_MICROPHONE,
        control_label: "Microphone",
    },
];

pub const SHORTCUT_DESCRIPTORS: &[ShortcutDescriptor] = &[
    ShortcutDescriptor {
        action: ShortcutAction::TogglePause,
        label: "Toggle pause",
    },
    ShortcutDescriptor {
        action: ShortcutAction::Reset,
        label: "Reset",
    },
    ShortcutDescriptor {
        action: ShortcutAction::SaveActiveSlotOrNew,
        label: "Save active slot",
    },
    ShortcutDescriptor {
        action: ShortcutAction::LoadActiveSlot,
        label: "Load active slot",
    },
    ShortcutDescriptor {
        action: ShortcutAction::SelectNextSlot,
        label: "Select next slot",
    },
    ShortcutDescriptor {
        action: ShortcutAction::SelectPreviousSlot,
        label: "Select previous slot",
    },
    ShortcutDescriptor {
        action: ShortcutAction::ToggleFullscreen,
        label: "Toggle fullscreen",
    },
];

pub const EDITABLE_KEYS: &[KeyboardKey] = &[
    KeyboardKey::KeyA,
    KeyboardKey::KeyB,
    KeyboardKey::KeyC,
    KeyboardKey::KeyD,
    KeyboardKey::KeyE,
    KeyboardKey::KeyF,
    KeyboardKey::KeyG,
    KeyboardKey::KeyH,
    KeyboardKey::KeyI,
    KeyboardKey::KeyJ,
    KeyboardKey::KeyK,
    KeyboardKey::KeyL,
    KeyboardKey::KeyM,
    KeyboardKey::KeyN,
    KeyboardKey::KeyO,
    KeyboardKey::KeyP,
    KeyboardKey::KeyQ,
    KeyboardKey::KeyR,
    KeyboardKey::KeyS,
    KeyboardKey::KeyT,
    KeyboardKey::KeyU,
    KeyboardKey::KeyV,
    KeyboardKey::KeyW,
    KeyboardKey::KeyX,
    KeyboardKey::KeyY,
    KeyboardKey::KeyZ,
    KeyboardKey::ArrowUp,
    KeyboardKey::ArrowDown,
    KeyboardKey::ArrowLeft,
    KeyboardKey::ArrowRight,
    KeyboardKey::Enter,
    KeyboardKey::Escape,
    KeyboardKey::Space,
    KeyboardKey::Tab,
    KeyboardKey::F1,
    KeyboardKey::F2,
    KeyboardKey::F3,
    KeyboardKey::F4,
    KeyboardKey::F5,
    KeyboardKey::F6,
    KeyboardKey::F7,
    KeyboardKey::F8,
    KeyboardKey::F9,
    KeyboardKey::F10,
    KeyboardKey::F11,
    KeyboardKey::F12,
];

pub fn default_desktop_settings() -> DesktopSettings {
    let mut settings = DesktopSettings {
        systems: BTreeMap::from([(SystemId::Nes, SystemSettings::Nes(NesSettings::default()))]),
        ..Default::default()
    };
    settings.input.keyboard_profiles.insert(
        SystemId::Nes,
        BindingProfile {
            bindings: vec![
                default_control_binding(NES_CONTROL_A, KeyboardKey::KeyZ),
                default_control_binding(NES_CONTROL_B, KeyboardKey::KeyX),
                default_control_binding(NES_CONTROL_SELECT, KeyboardKey::KeyC),
                default_control_binding(NES_CONTROL_START, KeyboardKey::KeyV),
                default_control_binding(NES_CONTROL_UP, KeyboardKey::ArrowUp),
                default_control_binding(NES_CONTROL_DOWN, KeyboardKey::ArrowDown),
                default_control_binding(NES_CONTROL_LEFT, KeyboardKey::ArrowLeft),
                default_control_binding(NES_CONTROL_RIGHT, KeyboardKey::ArrowRight),
            ],
        },
    );
    settings.shortcuts.keyboard = vec![
        ShortcutBinding {
            action: ShortcutAction::TogglePause,
            key: KeyboardKey::Space,
        },
        ShortcutBinding {
            action: ShortcutAction::Reset,
            key: KeyboardKey::Escape,
        },
        ShortcutBinding {
            action: ShortcutAction::SaveActiveSlotOrNew,
            key: KeyboardKey::F5,
        },
        ShortcutBinding {
            action: ShortcutAction::SelectNextSlot,
            key: KeyboardKey::F6,
        },
        ShortcutBinding {
            action: ShortcutAction::SelectPreviousSlot,
            key: KeyboardKey::F7,
        },
        ShortcutBinding {
            action: ShortcutAction::LoadActiveSlot,
            key: KeyboardKey::F8,
        },
        ShortcutBinding {
            action: ShortcutAction::ToggleFullscreen,
            key: KeyboardKey::F11,
        },
    ];
    settings
}

pub fn load_settings_manager() -> DesktopSettingsManager {
    let defaults = default_desktop_settings();
    match DesktopSettingsManager::load(defaults.clone()) {
        Ok(manager) => manager,
        Err(error) => {
            log::warn!("desktop settings file is unavailable; using in-memory defaults: {error}");
            DesktopSettingsManager::ephemeral(defaults)
        }
    }
}

pub fn build_screen_buffer(settings: &DesktopSettings) -> ScreenBuffer {
    ScreenBuffer::new_nes_gpu(filter_type(settings))
}

pub fn build_speaker(settings: &DesktopSettings) -> OpenAl {
    let buffer_width = settings.audio.buffer_size.max(32) as usize;
    let requested_sample_rate = settings.audio.sample_rate.max(8_000) as i32;
    let buffer_duration_ms =
        ((buffer_width as u32) * 1_000).div_ceil(settings.audio.sample_rate.max(1));
    let buffer_count = settings
        .audio
        .latency_ms
        .max(buffer_duration_ms)
        .div_ceil(buffer_duration_ms.max(1))
        .max(2) as usize;
    let gain = if settings.audio.muted {
        0.0
    } else {
        settings.audio.master_volume.clamp(0.0, 1.0)
    };
    OpenAl::with_gain(
        requested_sample_rate,
        CLOCK_RATE as i32,
        buffer_width,
        buffer_count,
        gain,
    )
}

pub fn effective_load_options(
    settings: &DesktopSettings,
    explicit: NesLoadOptions,
) -> NesLoadOptions {
    explicit.with_default_mmc3_irq_variant(system_settings(settings).core.mmc3_irq_variant)
}

pub fn controller_event_for_key(
    settings: &DesktopSettings,
    key: KeyboardKey,
    pressed: bool,
) -> Option<DigitalInputEvent> {
    let profile = settings.input.keyboard_profiles.get(&SystemId::Nes)?;
    profile
        .bindings
        .iter()
        .find_map(|binding| match &binding.source {
            HostInputSource::Keyboard(binding_key) if *binding_key == key => {
                nes_digital_event_from_binding(binding, pressed)
            }
            _ => None,
        })
}

pub fn shortcut_command_for_key(
    settings: &DesktopSettings,
    key: KeyboardKey,
) -> Option<SessionCommand> {
    shortcut_action_for_key(settings, key).and_then(shortcut_action_to_command)
}

pub fn shortcut_action_for_key(
    settings: &DesktopSettings,
    key: KeyboardKey,
) -> Option<ShortcutAction> {
    settings
        .shortcuts
        .keyboard
        .iter()
        .find(|binding| binding.key == key)
        .map(|binding| binding.action)
}

pub fn keyboard_key_label(key: KeyboardKey) -> &'static str {
    match key {
        KeyboardKey::KeyA => "A",
        KeyboardKey::KeyB => "B",
        KeyboardKey::KeyC => "C",
        KeyboardKey::KeyD => "D",
        KeyboardKey::KeyE => "E",
        KeyboardKey::KeyF => "F",
        KeyboardKey::KeyG => "G",
        KeyboardKey::KeyH => "H",
        KeyboardKey::KeyI => "I",
        KeyboardKey::KeyJ => "J",
        KeyboardKey::KeyK => "K",
        KeyboardKey::KeyL => "L",
        KeyboardKey::KeyM => "M",
        KeyboardKey::KeyN => "N",
        KeyboardKey::KeyO => "O",
        KeyboardKey::KeyP => "P",
        KeyboardKey::KeyQ => "Q",
        KeyboardKey::KeyR => "R",
        KeyboardKey::KeyS => "S",
        KeyboardKey::KeyT => "T",
        KeyboardKey::KeyU => "U",
        KeyboardKey::KeyV => "V",
        KeyboardKey::KeyW => "W",
        KeyboardKey::KeyX => "X",
        KeyboardKey::KeyY => "Y",
        KeyboardKey::KeyZ => "Z",
        KeyboardKey::Digit0 => "0",
        KeyboardKey::Digit1 => "1",
        KeyboardKey::Digit2 => "2",
        KeyboardKey::Digit3 => "3",
        KeyboardKey::Digit4 => "4",
        KeyboardKey::Digit5 => "5",
        KeyboardKey::Digit6 => "6",
        KeyboardKey::Digit7 => "7",
        KeyboardKey::Digit8 => "8",
        KeyboardKey::Digit9 => "9",
        KeyboardKey::ArrowUp => "Up",
        KeyboardKey::ArrowDown => "Down",
        KeyboardKey::ArrowLeft => "Left",
        KeyboardKey::ArrowRight => "Right",
        KeyboardKey::Enter => "Enter",
        KeyboardKey::Escape => "Escape",
        KeyboardKey::Space => "Space",
        KeyboardKey::Tab => "Tab",
        KeyboardKey::F1 => "F1",
        KeyboardKey::F2 => "F2",
        KeyboardKey::F3 => "F3",
        KeyboardKey::F4 => "F4",
        KeyboardKey::F5 => "F5",
        KeyboardKey::F6 => "F6",
        KeyboardKey::F7 => "F7",
        KeyboardKey::F8 => "F8",
        KeyboardKey::F9 => "F9",
        KeyboardKey::F10 => "F10",
        KeyboardKey::F11 => "F11",
        KeyboardKey::F12 => "F12",
    }
}

pub fn keyboard_key_id(key: KeyboardKey) -> &'static str {
    match key {
        KeyboardKey::KeyA => "key_a",
        KeyboardKey::KeyB => "key_b",
        KeyboardKey::KeyC => "key_c",
        KeyboardKey::KeyD => "key_d",
        KeyboardKey::KeyE => "key_e",
        KeyboardKey::KeyF => "key_f",
        KeyboardKey::KeyG => "key_g",
        KeyboardKey::KeyH => "key_h",
        KeyboardKey::KeyI => "key_i",
        KeyboardKey::KeyJ => "key_j",
        KeyboardKey::KeyK => "key_k",
        KeyboardKey::KeyL => "key_l",
        KeyboardKey::KeyM => "key_m",
        KeyboardKey::KeyN => "key_n",
        KeyboardKey::KeyO => "key_o",
        KeyboardKey::KeyP => "key_p",
        KeyboardKey::KeyQ => "key_q",
        KeyboardKey::KeyR => "key_r",
        KeyboardKey::KeyS => "key_s",
        KeyboardKey::KeyT => "key_t",
        KeyboardKey::KeyU => "key_u",
        KeyboardKey::KeyV => "key_v",
        KeyboardKey::KeyW => "key_w",
        KeyboardKey::KeyX => "key_x",
        KeyboardKey::KeyY => "key_y",
        KeyboardKey::KeyZ => "key_z",
        KeyboardKey::Digit0 => "digit_0",
        KeyboardKey::Digit1 => "digit_1",
        KeyboardKey::Digit2 => "digit_2",
        KeyboardKey::Digit3 => "digit_3",
        KeyboardKey::Digit4 => "digit_4",
        KeyboardKey::Digit5 => "digit_5",
        KeyboardKey::Digit6 => "digit_6",
        KeyboardKey::Digit7 => "digit_7",
        KeyboardKey::Digit8 => "digit_8",
        KeyboardKey::Digit9 => "digit_9",
        KeyboardKey::ArrowUp => "arrow_up",
        KeyboardKey::ArrowDown => "arrow_down",
        KeyboardKey::ArrowLeft => "arrow_left",
        KeyboardKey::ArrowRight => "arrow_right",
        KeyboardKey::Enter => "enter",
        KeyboardKey::Escape => "escape",
        KeyboardKey::Space => "space",
        KeyboardKey::Tab => "tab",
        KeyboardKey::F1 => "f1",
        KeyboardKey::F2 => "f2",
        KeyboardKey::F3 => "f3",
        KeyboardKey::F4 => "f4",
        KeyboardKey::F5 => "f5",
        KeyboardKey::F6 => "f6",
        KeyboardKey::F7 => "f7",
        KeyboardKey::F8 => "f8",
        KeyboardKey::F9 => "f9",
        KeyboardKey::F10 => "f10",
        KeyboardKey::F11 => "f11",
        KeyboardKey::F12 => "f12",
    }
}

pub fn keyboard_key_from_id(id: &str) -> Option<KeyboardKey> {
    EDITABLE_KEYS
        .iter()
        .copied()
        .find(|key| keyboard_key_id(*key) == id)
}

pub fn keyboard_binding_descriptors() -> &'static [NesKeyboardBindingDescriptor] {
    NES_KEYBOARD_BINDINGS
}

pub fn shortcut_descriptors() -> &'static [ShortcutDescriptor] {
    SHORTCUT_DESCRIPTORS
}

pub fn system_settings(settings: &DesktopSettings) -> NesSettings {
    settings
        .systems
        .get(&SystemId::Nes)
        .map(|settings| match settings {
            SystemSettings::Nes(nes) => nes.clone(),
        })
        .unwrap_or_default()
}

pub fn filter_type(settings: &DesktopSettings) -> FilterType {
    match system_settings(settings).video.filter {
        NesVideoFilter::None => FilterType::None,
        NesVideoFilter::NtscRgb => FilterType::NtscRGB,
        NesVideoFilter::NtscComposite => FilterType::NtscComposite,
        NesVideoFilter::NtscSVideo => FilterType::NtscSVideo,
    }
}

pub fn current_or_default(manager: &DesktopSettingsManager) -> DesktopSettings {
    manager.current().unwrap_or_else(|error| {
        log::warn!("desktop settings read failed; using defaults: {error}");
        default_desktop_settings()
    })
}

pub fn save_settings(
    manager: &DesktopSettingsManager,
    settings: DesktopSettings,
) -> Result<(), SettingsError> {
    manager.save(settings)
}

fn default_control_binding(control: DigitalControlId, key: KeyboardKey) -> ControlBinding {
    ControlBinding {
        attachment: PersistedAttachmentId::new(NES_ATTACHMENT_PLAYER_ONE.as_str()),
        control: PersistedControlId::digital(control.as_str()),
        source: HostInputSource::Keyboard(key),
    }
}

fn nes_digital_event_from_binding(
    binding: &ControlBinding,
    pressed: bool,
) -> Option<DigitalInputEvent> {
    let attachment = match binding.attachment.as_str() {
        value if value == NES_ATTACHMENT_PLAYER_ONE.as_str() => NES_ATTACHMENT_PLAYER_ONE,
        value if value == NES_ATTACHMENT_PLAYER_TWO.as_str() => NES_ATTACHMENT_PLAYER_TWO,
        _ => return None,
    };
    let control = match (attachment, binding.control.as_str()) {
        (_, value) if value == NES_CONTROL_A.as_str() => NES_CONTROL_A,
        (_, value) if value == NES_CONTROL_B.as_str() => NES_CONTROL_B,
        (NES_ATTACHMENT_PLAYER_ONE, value) if value == NES_CONTROL_SELECT.as_str() => {
            NES_CONTROL_SELECT
        }
        (NES_ATTACHMENT_PLAYER_ONE, value) if value == NES_CONTROL_START.as_str() => {
            NES_CONTROL_START
        }
        (_, value) if value == NES_CONTROL_UP.as_str() => NES_CONTROL_UP,
        (_, value) if value == NES_CONTROL_DOWN.as_str() => NES_CONTROL_DOWN,
        (_, value) if value == NES_CONTROL_LEFT.as_str() => NES_CONTROL_LEFT,
        (_, value) if value == NES_CONTROL_RIGHT.as_str() => NES_CONTROL_RIGHT,
        (NES_ATTACHMENT_PLAYER_TWO, value) if value == FAMICOM_P2_CONTROL_MICROPHONE.as_str() => {
            FAMICOM_P2_CONTROL_MICROPHONE
        }
        _ => return None,
    };
    Some(DigitalInputEvent::new(
        attachment,
        control,
        if pressed {
            nerust_input_schema::DigitalInputState::Pressed
        } else {
            nerust_input_schema::DigitalInputState::Released
        },
    ))
}

fn shortcut_action_to_command(action: ShortcutAction) -> Option<SessionCommand> {
    Some(match action {
        ShortcutAction::TogglePause => SessionCommand::TogglePause,
        ShortcutAction::Reset => SessionCommand::Reset,
        ShortcutAction::SaveActiveSlotOrNew => SessionCommand::SaveActiveSlotOrNew,
        ShortcutAction::LoadActiveSlot => SessionCommand::LoadActiveSlot,
        ShortcutAction::SelectNextSlot => SessionCommand::SelectNextSlot,
        ShortcutAction::SelectPreviousSlot => SessionCommand::SelectPreviousSlot,
        ShortcutAction::ToggleFullscreen => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        controller_event_for_key, current_or_default, default_desktop_settings,
        effective_load_options, filter_type, keyboard_key_label, shortcut_action_for_key,
        shortcut_command_for_key,
    };
    use crate::load::{NesLoadOptions, NesMmc3IrqVariant};
    use nerust_contract_options::Mmc3IrqVariant;
    use nerust_contract_settings::{
        desktop::SystemSettings, input::KeyboardKey, nes::NesVideoFilter, shortcut::ShortcutAction,
    };
    use nerust_gui_runtime::settings::DesktopSettingsManager;
    use nerust_gui_session::commands::SessionCommand;
    use nerust_input_nes::topology::{
        FAMICOM_P2_CONTROL_MICROPHONE, NES_ATTACHMENT_PLAYER_ONE, NES_ATTACHMENT_PLAYER_TWO,
        NES_CONTROL_A,
    };

    #[test]
    fn default_settings_seed_nes_bindings_and_system_settings() {
        let settings = default_desktop_settings();

        assert!(
            settings
                .systems
                .contains_key(&nerust_input_schema::SystemId::Nes)
        );
        assert!(
            settings
                .input
                .keyboard_profiles
                .contains_key(&nerust_input_schema::SystemId::Nes)
        );
    }

    #[test]
    fn keyboard_bindings_resolve_to_nes_input_events() {
        let settings = default_desktop_settings();
        let event = controller_event_for_key(&settings, KeyboardKey::KeyZ, true).unwrap();

        assert_eq!(event.attachment, NES_ATTACHMENT_PLAYER_ONE);
        assert_eq!(event.control, NES_CONTROL_A);
    }

    #[test]
    fn keyboard_bindings_support_player_two_controls() {
        let mut settings = default_desktop_settings();
        settings
            .input
            .keyboard_profiles
            .get_mut(&nerust_input_schema::SystemId::Nes)
            .unwrap()
            .bindings
            .push(nerust_contract_settings::input::ControlBinding {
                attachment: nerust_contract_settings::input::PersistedAttachmentId::new(
                    NES_ATTACHMENT_PLAYER_TWO.as_str(),
                ),
                control: nerust_contract_settings::input::PersistedControlId::digital(
                    FAMICOM_P2_CONTROL_MICROPHONE.as_str(),
                ),
                source: nerust_contract_settings::input::HostInputSource::Keyboard(
                    KeyboardKey::KeyM,
                ),
            });
        let event = controller_event_for_key(&settings, KeyboardKey::KeyM, true).unwrap();

        assert_eq!(event.attachment, NES_ATTACHMENT_PLAYER_TWO);
        assert_eq!(event.control, FAMICOM_P2_CONTROL_MICROPHONE);
    }

    #[test]
    fn shortcuts_resolve_to_session_commands() {
        let settings = default_desktop_settings();

        assert_eq!(
            shortcut_command_for_key(&settings, KeyboardKey::F5),
            Some(SessionCommand::SaveActiveSlotOrNew)
        );
    }

    #[test]
    fn fullscreen_shortcut_is_exposed_as_action() {
        let settings = default_desktop_settings();

        assert_eq!(
            shortcut_action_for_key(&settings, KeyboardKey::F11),
            Some(ShortcutAction::ToggleFullscreen)
        );
        assert_eq!(shortcut_command_for_key(&settings, KeyboardKey::F11), None);
    }

    #[test]
    fn explicit_load_options_win_over_saved_defaults() {
        let mut settings = default_desktop_settings();
        let SystemSettings::Nes(nes) = settings
            .systems
            .get_mut(&nerust_input_schema::SystemId::Nes)
            .unwrap();
        nes.core.mmc3_irq_variant = Some(Mmc3IrqVariant::Sharp);

        let resolved = effective_load_options(
            &settings,
            NesLoadOptions {
                mmc3_irq_variant: Some(NesMmc3IrqVariant::Nec),
            },
        );

        assert_eq!(resolved.mmc3_irq_variant, Some(NesMmc3IrqVariant::Nec));
    }

    #[test]
    fn saved_nes_filter_maps_to_screen_filter_type() {
        let mut settings = default_desktop_settings();
        let SystemSettings::Nes(nes) = settings
            .systems
            .get_mut(&nerust_input_schema::SystemId::Nes)
            .unwrap();
        nes.video.filter = NesVideoFilter::NtscSVideo;

        assert!(matches!(
            filter_type(&settings),
            nerust_screen_filter::FilterType::NtscSVideo
        ));
    }

    #[test]
    fn keyboard_key_labels_are_human_readable() {
        assert_eq!(keyboard_key_label(KeyboardKey::ArrowUp), "Up");
    }

    #[test]
    fn current_or_default_falls_back_for_ephemeral_manager_reads() {
        let manager = DesktopSettingsManager::ephemeral(default_desktop_settings());
        assert!(
            current_or_default(&manager)
                .systems
                .contains_key(&nerust_input_schema::SystemId::Nes)
        );
    }
}

//! Display composition for ports, slots, and pad buttons.
//!
//! The settings core owns the locale string table; composition that names
//! HID topology (ports, slots, pad caps) lives here, next to the DTOs it
//! renders. Frontends call these instead of formatting core-side.

use nerust_input_traits::StandardButton;
use nerust_settings_core::i18n::{I18n, UiText};
use nerust_settings_core::model::language::AppLanguage;

/// Display text for one pad button in mapping rows: cap print plus a
/// plain position for the four face buttons (`A (bottom)`), cap alone
/// elsewhere. Caps follow the dominant print and the Android `BUTTON_*`
/// names; the position word keeps Nintendo-layout pads interpretable
/// without per-vendor branches.
/// Plain cap print for one pad button: the single home for the
/// position-to-glyph table. Positions are the binding identity
/// (vendors transpose caps); this table only names what is printed.
pub fn pad_button_plain(button: StandardButton) -> &'static str {
    match button {
        StandardButton::South => "A",
        StandardButton::East => "B",
        StandardButton::West => "X",
        StandardButton::North => "Y",
        StandardButton::Back => "Back",
        StandardButton::Guide => "Guide",
        StandardButton::Start => "Start",
        StandardButton::LeftStick => "L3",
        StandardButton::RightStick => "R3",
        StandardButton::LeftShoulder => "LB",
        StandardButton::RightShoulder => "RB",
        StandardButton::LeftTrigger => "LT",
        StandardButton::RightTrigger => "RT",
        StandardButton::DpadUp => "D-Up",
        StandardButton::DpadDown => "D-Down",
        StandardButton::DpadLeft => "D-Left",
        StandardButton::DpadRight => "D-Right",
    }
}

pub fn pad_button_label(i18n: I18n, button: StandardButton) -> String {
    use StandardButton as Pad;
    let position = match button {
        Pad::South => Some(UiText::PosBottom),
        Pad::East => Some(UiText::PosRight),
        Pad::West => Some(UiText::PosLeft),
        Pad::North => Some(UiText::PosTop),
        _ => None,
    };
    match position {
        Some(place) => format!("{} ({})", pad_button_plain(button), i18n.text(place)),
        None => pad_button_plain(button).to_string(),
    }
}

/// Player/device-row summary: "Not assigned" when empty, otherwise
/// "Name" or "Name +N" (short form is banned: the full first name reads).
pub fn port_summary(i18n: I18n, names: &[String]) -> String {
    match names {
        [] => i18n.text(UiText::NotAssigned).to_string(),
        [first] => first.clone(),
        // Count suffix is language-neutral, so no per-language branch.
        [first, rest @ ..] => format!("{first} +{}", rest.len()),
    }
}

/// Player/device-row count line: "No devices assigned" / "1 device
/// assigned" / "N devices assigned" (singular/plural distinguished).
pub fn port_count_line(i18n: I18n, count: usize) -> String {
    match (i18n.language(), count) {
        (AppLanguage::Japanese, 0) => "割り当てデバイスなし".to_string(),
        (_, 0) => "No devices assigned".to_string(),
        (AppLanguage::Japanese, 1) => "1台のデバイスを割り当て".to_string(),
        (_, 1) => "1 device assigned".to_string(),
        (AppLanguage::Japanese, n) => format!("{n}台のデバイスを割り当て"),
        (_, n) => format!("{n} devices assigned"),
    }
}

/// Connection-order pad label (`Pad 1` / `パッド 1`). The number is
/// connection order, never the player slot, in both languages.
pub fn pad_label(language: AppLanguage, index: usize) -> String {
    let pad = index + 1;
    match language {
        AppLanguage::Japanese => format!("パッド {pad}"),
        _ => format!("Pad {pad}"),
    }
}

/// Description for a controller-slot choice row: which port the row drives
/// is in the row label, so the description only states the action.
pub fn slot_description(i18n: I18n) -> String {
    match i18n.language() {
        AppLanguage::Japanese => "このポートで使うコントローラー".to_string(),
        _ => "Controller for this port.".to_string(),
    }
}

/// Full disabled-choice sentence, formatted here so word order
/// follows the locale.
pub fn slot_in_use(i18n: I18n, owner: &str) -> String {
    match i18n.language() {
        AppLanguage::Japanese => format!("{owner}使用中"),
        _ => format!("Used in {owner}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nerust_settings_core::i18n::I18n;
    use nerust_settings_core::model::language::AppLanguage;

    fn en() -> I18n {
        I18n::new(AppLanguage::English)
    }

    fn ja() -> I18n {
        I18n::new(AppLanguage::Japanese)
    }

    #[test]
    fn pad_buttons_pair_cap_with_plain_position() {
        use StandardButton as Pad;
        assert_eq!(pad_button_label(en(), Pad::South), "A (bottom)");
        assert_eq!(pad_button_label(ja(), Pad::South), "A (下)");
        assert_eq!(pad_button_label(en(), Pad::North), "Y (top)");
        assert_eq!(pad_button_label(en(), Pad::Guide), "Guide");
        assert_eq!(pad_button_label(ja(), Pad::LeftShoulder), "LB");
    }

    #[test]
    fn summaries_cover_empty_single_and_multi() {
        assert_eq!(port_summary(en(), &[]), "Not assigned");
        assert_eq!(port_summary(ja(), &[]), "未割り当て");
        assert_eq!(
            port_summary(en(), &["Pad 1".to_string()]),
            "Pad 1".to_string()
        );
        assert_eq!(
            port_summary(en(), &["A".to_string(), "B".to_string()]),
            "A +1".to_string()
        );
        assert_eq!(port_count_line(en(), 0), "No devices assigned");
        assert_eq!(port_count_line(ja(), 2), "2台のデバイスを割り当て");
        assert_eq!(slot_in_use(ja(), "P1"), "P1使用中");
        assert_eq!(slot_in_use(en(), "P1"), "Used in P1");
    }
}

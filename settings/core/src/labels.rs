//! Display composition for ports, slots, and pad buttons.
//!
//! The settings core owns the locale string table; composition that names
//! HID topology (ports, slots, pad caps) lives here so viewmodels and
//! frontends format through one home instead of each owning a copy.

use nerust_input_traits::StandardButton;

use crate::{
    i18n::{I18n, UiText},
    model::language::AppLanguage,
};

/// Plain cap print for one pad button: the single home for the
/// position-to-glyph table. Positions are the binding identity
/// (vendors transpose caps); this table only names what is printed.
fn pad_button_plain(button: StandardButton) -> &'static str {
    match button {
        StandardButton::South => "A",
        StandardButton::East => "B",
        StandardButton::West => "X",
        StandardButton::North => "Y",
        StandardButton::Back => "Back",
        StandardButton::Guide => "Guide",
        StandardButton::Start => "Start",
        StandardButton::Capture => "Capture",
        StandardButton::LeftStick => "L3",
        StandardButton::RightStick => "R3",
        StandardButton::LeftShoulder => "LB",
        StandardButton::RightShoulder => "RB",
        StandardButton::LeftTrigger => "LT",
        StandardButton::RightTrigger => "RT",
        StandardButton::DpadUp => "D-pad up",
        StandardButton::DpadDown => "D-pad down",
        StandardButton::DpadLeft => "D-pad left",
        StandardButton::DpadRight => "D-pad right",
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

/// Connection-order pad label (`Gamepad 1` / `ゲームパッド 1`). The number
/// is connection order, never the player slot, in both languages.
pub fn pad_label(i18n: I18n, index: usize) -> String {
    let pad = index + 1;
    match i18n.language() {
        AppLanguage::Japanese => format!("ゲームパッド {pad}"),
        _ => format!("Gamepad {pad}"),
    }
}

/// Description for a controller-slot choice row: which port the row drives
/// is in the row label, so the description only states the action.
pub fn slot_description(i18n: I18n) -> String {
    match i18n.language() {
        AppLanguage::Japanese => "このプレイヤーで使うコントローラー".to_string(),
        _ => "Controller for this player.".to_string(),
    }
}

/// Full disabled-choice sentence, formatted here so word order
/// follows the locale.
pub fn slot_in_use(i18n: I18n, owner: &str) -> String {
    match i18n.language() {
        AppLanguage::Japanese => format!("{owner}使用中"),
        _ => format!("In use by {owner}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            port_summary(en(), &["Gamepad 1".to_string()]),
            "Gamepad 1".to_string()
        );
        assert_eq!(
            port_summary(en(), &["A".to_string(), "B".to_string()]),
            "A +1".to_string()
        );
        assert_eq!(slot_in_use(ja(), "P1"), "P1使用中");
        assert_eq!(slot_in_use(en(), "P1"), "In use by P1");
    }
}

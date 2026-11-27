//! Display composition for ports, slots, and pad buttons.
//!
//! The settings core owns the locale string table; composition that names
//! HID topology (ports, slots, pad caps) lives here so viewmodels and
//! frontends format through one home instead of each owning a copy.

use nerust_input_traits::{PadLayout, StandardButton};

use crate::{
    i18n::{I18n, UiText},
    model::language::AppLanguage,
};

/// Plain cap print for one pad button: the single home for the
/// position-to-glyph table. Positions are the binding identity
/// (vendors transpose caps); this table only names what is printed.
fn pad_button_plain(button: StandardButton, layout: PadLayout) -> &'static str {
    use StandardButton as Pad;
    match (button, layout) {
        // Sony symbols are printed on the pad; language-independent.
        (Pad::South, PadLayout::Sony) => "×",
        (Pad::East, PadLayout::Sony) => "○",
        (Pad::West, PadLayout::Sony) => "□",
        (Pad::North, PadLayout::Sony) => "△",
        // Nintendo diamond (the emulated platforms): B bottom, A right.
        (Pad::South, PadLayout::Nintendo) => "B",
        (Pad::East, PadLayout::Nintendo) => "A",
        (Pad::West, PadLayout::Nintendo) => "Y",
        (Pad::North, PadLayout::Nintendo) => "X",
        // Xbox diamond and the fallback for unknown vendors.
        (Pad::South, PadLayout::Xbox) => "A",
        (Pad::East, PadLayout::Xbox) => "B",
        (Pad::West, PadLayout::Xbox) => "X",
        (Pad::North, PadLayout::Xbox) => "Y",
        (Pad::Back, _) => "Back",
        (Pad::Guide, _) => "Guide",
        (Pad::Start, _) => "Start",
        (Pad::Capture, _) => "Capture",
        (Pad::LeftStick, _) => "L3",
        (Pad::RightStick, _) => "R3",
        (Pad::LeftShoulder, _) => "LB",
        (Pad::RightShoulder, _) => "RB",
        (Pad::LeftTrigger, _) => "LT",
        (Pad::RightTrigger, _) => "RT",
        (Pad::DpadUp, _) => "D-pad up",
        (Pad::DpadDown, _) => "D-pad down",
        (Pad::DpadLeft, _) => "D-pad left",
        (Pad::DpadRight, _) => "D-pad right",
    }
}

pub fn pad_button_label(_i18n: I18n, button: StandardButton, layout: PadLayout) -> String {
    pad_button_plain(button, layout).to_string()
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
    fn pad_buttons_follow_vendor_layout() {
        use StandardButton as Pad;
        use nerust_input_traits::PadLayout as Layout;
        assert_eq!(pad_button_label(en(), Pad::South, Layout::Sony), "×");
        assert_eq!(pad_button_label(en(), Pad::East, Layout::Sony), "○");
        assert_eq!(pad_button_label(ja(), Pad::South, Layout::Sony), "×");
        assert_eq!(pad_button_label(en(), Pad::South, Layout::Nintendo), "B");
        assert_eq!(pad_button_label(en(), Pad::East, Layout::Nintendo), "A");
        assert_eq!(pad_button_label(ja(), Pad::East, Layout::Nintendo), "A");
        assert_eq!(pad_button_label(en(), Pad::South, Layout::Xbox), "A");
        assert_eq!(pad_button_label(en(), Pad::East, Layout::Xbox), "B");
        assert_eq!(pad_button_label(en(), Pad::Guide, Layout::Sony), "Guide");
        assert_eq!(
            pad_button_label(ja(), Pad::LeftShoulder, Layout::Nintendo),
            "LB"
        );
    }

    #[test]
    fn layout_for_vendor_pins_sony_nintendo_and_fallback() {
        use nerust_input_traits::PadLayout as Layout;
        assert_eq!(Layout::for_vendor(Some(0x054C)), Layout::Sony);
        assert_eq!(Layout::for_vendor(Some(0x057E)), Layout::Nintendo);
        assert_eq!(Layout::for_vendor(Some(0x045E)), Layout::Xbox);
        assert_eq!(Layout::for_vendor(None), Layout::Xbox);
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

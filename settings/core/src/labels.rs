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

/// Cap print for one pad button: the single home for the
/// position-to-glyph table. Positions are the binding identity
/// (vendors transpose caps); this table only names what is printed.
pub fn pad_cap(button: StandardButton, layout: PadLayout) -> &'static str {
    use StandardButton as Pad;
    match button {
        Pad::South => match layout {
            PadLayout::Sony => "×",
            PadLayout::Nintendo => "B",
            PadLayout::Xbox => "A",
        },
        Pad::East => match layout {
            PadLayout::Sony => "○",
            PadLayout::Nintendo => "A",
            PadLayout::Xbox => "B",
        },
        Pad::West => match layout {
            PadLayout::Sony => "□",
            PadLayout::Nintendo => "Y",
            PadLayout::Xbox => "X",
        },
        Pad::North => match layout {
            PadLayout::Sony => "△",
            PadLayout::Nintendo => "X",
            PadLayout::Xbox => "Y",
        },
        Pad::Back => "Back",
        Pad::Guide => "Guide",
        Pad::Start => "Start",
        Pad::Capture => "Capture",
        Pad::LeftStick => "L3",
        Pad::RightStick => "R3",
        Pad::LeftShoulder => "LB",
        Pad::RightShoulder => "RB",
        Pad::LeftTrigger => "LT",
        Pad::RightTrigger => "RT",
        Pad::DpadUp => "D-pad up",
        Pad::DpadDown => "D-pad down",
        Pad::DpadLeft => "D-pad left",
        Pad::DpadRight => "D-pad right",
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

/// Pad heading: the OS device name when the rank is connected, else the
/// rank label above. Ranks are stable (disconnects keep their order
/// entry), so the fallback never misnames a reconnect.
pub fn pad_display_name(i18n: I18n, index: usize, name: &str) -> String {
    if name.is_empty() {
        pad_label(i18n, index)
    } else {
        name.to_string()
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

    #[test]
    fn pad_display_name_prefers_device_name() {
        assert_eq!(
            pad_display_name(en(), 0, "Wireless Controller"),
            "Wireless Controller"
        );
        assert_eq!(pad_display_name(en(), 0, ""), "Gamepad 1");
        assert_eq!(
            pad_display_name(I18n::new(AppLanguage::Japanese), 1, ""),
            "ゲームパッド 2"
        );
    }

    fn ja() -> I18n {
        I18n::new(AppLanguage::Japanese)
    }

    #[test]
    fn pad_buttons_follow_vendor_layout() {
        use StandardButton as Pad;
        use nerust_input_traits::PadLayout as Layout;
        assert_eq!(pad_cap(Pad::South, Layout::Sony), "×");
        assert_eq!(pad_cap(Pad::East, Layout::Sony), "○");
        assert_eq!(pad_cap(Pad::South, Layout::Nintendo), "B");
        assert_eq!(pad_cap(Pad::East, Layout::Nintendo), "A");
        assert_eq!(pad_cap(Pad::South, Layout::Xbox), "A");
        assert_eq!(pad_cap(Pad::East, Layout::Xbox), "B");
        assert_eq!(pad_cap(Pad::Guide, Layout::Sony), "Guide");
        assert_eq!(pad_cap(Pad::LeftShoulder, Layout::Nintendo), "LB");
    }

    #[test]
    fn layout_for_vendor_pins_sony_nintendo_and_fallback() {
        use nerust_input_traits::PadLayout as Layout;
        use nerust_input_traits::{NINTENDO_VENDOR_ID, SONY_VENDOR_ID};
        assert_eq!(Layout::for_vendor(Some(SONY_VENDOR_ID)), Layout::Sony);
        assert_eq!(
            Layout::for_vendor(Some(NINTENDO_VENDOR_ID)),
            Layout::Nintendo
        );
        assert_eq!(Layout::for_vendor(Some(0x045E)), Layout::Xbox);
        assert_eq!(Layout::for_vendor(None), Layout::Xbox);
        assert_eq!(Layout::default(), Layout::Xbox);
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

//! Settings section taxonomy owned by the settings domain.
//! Rust owns the id→label mapping; Kotlin renders the pushed label only.
//!
//! Top-level taxonomy is purpose-based (zero-based IA): machine sections
//! hide under Advanced, controller screens hide behind player ports.

use nerust_settings_core::i18n::{I18n, UiText, text};

/// Canonical settings section ids in payload order: visible top-level
/// first, hidden navigation targets after.
pub(crate) const SETTINGS_SECTION_IDS: &[&str] = &[
    "play",
    "controllers",
    "saves",
    "notifications",
    "app",
    "advanced",
    "player.1",
    "player.2",
    "types.1",
    "types.2",
    "devices.p1",
    "devices.p2",
    "controls.overlay_detail",
    "controls.shortcuts",
    "audio.detail",
    "about",
    "system.nes",
    "system.gbc",
    "system.gba",
    "input.nes",
    "input.gbc",
    "input.gba",
    "keys.nes",
    "keys.gbc",
    "keys.gba",
];

/// Canonical section id→label table, returned as static strings.
/// Unknown ids fall back to a static rather than a formatted guess.
/// System names are proper nouns, identical in both languages.
pub(crate) fn section_label(id: &str, i18n: I18n) -> &'static str {
    let language = i18n.language();
    match id {
        "play" => text(language, UiText::SectionPlay),
        "controllers" => text(language, UiText::SectionControllers),
        "saves" => text(language, UiText::SectionSaves),
        "notifications" => text(language, UiText::Notifications),
        "app" => text(language, UiText::SectionApp),
        "advanced" => text(language, UiText::SectionAdvanced),
        "system.nes" => "Nintendo Entertainment System",
        "system.gbc" => "Game Boy Color",
        "system.gba" => "Game Boy Advance",
        _ => text(language, UiText::Unknown),
    }
}

#[cfg(test)]
mod tests {
    use super::{SETTINGS_SECTION_IDS, section_label};
    use nerust_settings_core::i18n::I18n;
    use nerust_settings_core::model::language::AppLanguage;
    use std::collections::BTreeSet;

    #[test]
    fn section_ids_stay_unique_with_static_labels() {
        for language in [AppLanguage::English, AppLanguage::Japanese] {
            let mut seen = BTreeSet::new();
            for id in SETTINGS_SECTION_IDS {
                assert!(seen.insert(*id), "duplicate section id {id}");
                assert!(!section_label(id, I18n::new(language)).is_empty());
            }
            assert_ne!(
                section_label("play", I18n::new(AppLanguage::English)),
                section_label("play", I18n::new(AppLanguage::Japanese))
            );
            assert_eq!(
                section_label("system.gbc", I18n::new(language)),
                "Game Boy Color"
            );
            assert_eq!(
                section_label("foo_bar", I18n::new(AppLanguage::English)),
                "Unknown"
            );
        }
        assert_eq!(
            section_label("play", I18n::new(AppLanguage::English)),
            "Play"
        );
        assert_eq!(
            section_label("play", I18n::new(AppLanguage::Japanese)),
            "プレイ"
        );
        assert_eq!(
            section_label("controllers", I18n::new(AppLanguage::Japanese)),
            "コントローラー"
        );
        assert_eq!(
            section_label("notifications", I18n::new(AppLanguage::English)),
            "Notifications"
        );
        assert_eq!(section_label("app", I18n::new(AppLanguage::English)), "App");
        assert_eq!(
            section_label("app", I18n::new(AppLanguage::Japanese)),
            "アプリ"
        );
        assert_eq!(
            section_label("system.unknown", I18n::new(AppLanguage::English)),
            "Unknown"
        );
        assert_eq!(
            section_label("foo_bar", I18n::new(AppLanguage::Japanese)),
            "不明"
        );
    }
}

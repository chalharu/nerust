//! Settings section taxonomy owned by the settings domain.
//! Rust owns the id→label mapping; Kotlin renders the pushed label only.

use nerust_settings_core::i18n::{UiText, text as ui_text};
use nerust_settings_core::model::language::AppLanguage;

/// Canonical settings section ids in payload order.
pub(crate) const SETTINGS_SECTION_IDS: &[&str] = &[
    "general",
    "audio",
    "video",
    "controls",
    "storage",
    "notifications",
    "system.nes",
    "system.gbc",
    "system.gba",
];

/// Canonical section id→label table, returned as static strings.
/// Unknown ids fall back to a static rather than a formatted guess.
/// System names are proper nouns, identical in both languages.
pub(crate) fn section_label(id: &str, language: AppLanguage) -> &'static str {
    match id {
        "audio" => ui_text(language, UiText::Audio),
        "video" => ui_text(language, UiText::Video),
        "controls" => ui_text(language, UiText::Controls),
        "storage" => ui_text(language, UiText::Storage),
        "notifications" => ui_text(language, UiText::Notifications),
        "general" => ui_text(language, UiText::General),
        "system.nes" => "Nintendo Entertainment System",
        "system.gbc" => "Game Boy Color",
        "system.gba" => "Game Boy Advance",
        _ => ui_text(language, UiText::Unknown),
    }
}

#[cfg(test)]
mod tests {
    use super::{SETTINGS_SECTION_IDS, section_label};
    use nerust_settings_core::model::language::AppLanguage;
    use std::collections::BTreeSet;

    #[test]
    fn section_ids_stay_unique_with_static_labels() {
        for language in [AppLanguage::English, AppLanguage::Japanese] {
            let mut seen = BTreeSet::new();
            for id in SETTINGS_SECTION_IDS {
                assert!(seen.insert(*id), "duplicate section id {id}");
                assert!(!section_label(id, language).is_empty());
            }
            assert_ne!(
                section_label("audio", AppLanguage::English),
                section_label("audio", AppLanguage::Japanese)
            );
            assert_eq!(section_label("system.gbc", language), "Game Boy Color");
            assert_eq!(section_label("foo_bar", AppLanguage::English), "Unknown");
        }
        assert_eq!(section_label("audio", AppLanguage::English), "Audio");
        assert_eq!(section_label("audio", AppLanguage::Japanese), "音声");
        assert_eq!(section_label("controls", AppLanguage::Japanese), "操作");
        assert_eq!(
            section_label("notifications", AppLanguage::English),
            "Notifications"
        );
        assert_eq!(section_label("general", AppLanguage::English), "General");
        assert_eq!(section_label("general", AppLanguage::Japanese), "一般");
        assert_eq!(
            section_label("system.unknown", AppLanguage::English),
            "Unknown"
        );
        assert_eq!(section_label("foo_bar", AppLanguage::Japanese), "不明");
    }
}

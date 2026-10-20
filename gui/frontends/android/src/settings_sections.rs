//! Settings section taxonomy owned by the settings domain.
//! Rust owns the id→label mapping; Kotlin renders the pushed label only.

/// Canonical settings section ids in payload order.
pub(crate) const SETTINGS_SECTION_IDS: &[&str] = &[
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
pub(crate) fn section_label(id: &str) -> &'static str {
    match id {
        "audio" => "Audio",
        "video" => "Video",
        "controls" => "Controls",
        "storage" => "Storage",
        "notifications" => "Notifications",
        "general" => "General",
        "system.nes" => "Nintendo Entertainment System",
        "system.gbc" => "Game Boy Color",
        "system.gba" => "Game Boy Advance",
        _ => "Unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::{SETTINGS_SECTION_IDS, section_label};
    use std::collections::BTreeSet;

    #[test]
    fn section_ids_stay_unique_with_static_labels() {
        let mut seen = BTreeSet::new();
        for id in SETTINGS_SECTION_IDS {
            assert!(seen.insert(*id), "duplicate section id {id}");
            assert!(!section_label(id).is_empty());
        }
        assert_eq!(section_label("audio"), "Audio");
        assert_eq!(section_label("notifications"), "Notifications");
        assert_eq!(section_label("system.gbc"), "Game Boy Color");
        assert_eq!(section_label("general"), "General");
        assert_eq!(section_label("system.unknown"), "Unknown");
        assert_eq!(section_label("foo_bar"), "Unknown");
    }
}

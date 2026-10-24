pub(crate) fn resolve(label_id: &str, language: &str) -> Option<String> {
    let (en, ja) = match label_id {
        "gba.peripheral.solar_light" => ("Solar Light", "ソーラー光量"),
        "gba.solar.blinding" => ("Blinding", "直射"),
        "gba.solar.bright" => ("Bright", "明るい"),
        "gba.solar.normal" => ("Normal", "普通"),
        "gba.solar.dim" => ("Dim", "暗い"),
        "gba.solar.dark" => ("Dark", "暗闇"),
        _ => return None,
    };
    Some(if language == "ja" { ja } else { en }.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_english_and_japanese_labels() {
        assert_eq!(
            resolve("gba.peripheral.solar_light", "en").as_deref(),
            Some("Solar Light")
        );
        assert_eq!(
            resolve("gba.peripheral.solar_light", "ja").as_deref(),
            Some("ソーラー光量")
        );
        assert_eq!(resolve("gba.solar.dark", "en").as_deref(), Some("Dark"));
        assert!(resolve("unknown", "en").is_none());
    }

    #[test]
    fn resolves_none_for_unknown_id() {
        assert!(resolve("unknown", "en").is_none());
        assert!(resolve("gba.system.unknown", "ja").is_none());
    }
}

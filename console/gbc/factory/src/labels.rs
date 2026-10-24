pub(crate) fn resolve(label_id: &str, language: &str) -> Option<String> {
    let (en, ja) = match label_id {
        "gbc.system.hardware_model" => ("Hardware Model", "ハードウェアモデル"),
        "gbc.hardware.dmg0" => ("Game Boy (DMG0)", "ゲームボーイ (DMG0)"),
        "gbc.hardware.dmg" => ("Game Boy (DMG)", "ゲームボーイ (DMG)"),
        "gbc.hardware.cgb_c" => ("Game Boy Color (CGB-C)", "ゲームボーイカラー (CGB-C)"),
        "gbc.hardware.cgb_d" => ("Game Boy Color (CGB-D)", "ゲームボーイカラー (CGB-D)"),
        "gbc.hardware.agb" => ("Game Boy Advance (AGB)", "ゲームボーイアドバンス (AGB)"),
        "gbc.system.rtc_sync" => ("RTC Sync", "RTC同期"),
        "gbc.rtc_sync.off" => ("Off", "同期しない"),
        "gbc.rtc_sync.save_data_only" => ("Save Data Only", "通常セーブのみ"),
        "gbc.rtc_sync.system_time" => ("Save Data + Snapshots", "通常セーブとSnapshot"),
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
            resolve("gbc.system.hardware_model", "en").as_deref(),
            Some("Hardware Model")
        );
        assert_eq!(
            resolve("gbc.system.hardware_model", "ja").as_deref(),
            Some("ハードウェアモデル")
        );
        assert_eq!(
            resolve("gbc.rtc_sync.save_data_only", "en").as_deref(),
            Some("Save Data Only")
        );
        assert_eq!(
            resolve("gbc.rtc_sync.system_time", "ja").as_deref(),
            Some("通常セーブとSnapshot")
        );
        assert!(resolve("unknown", "en").is_none());
    }
}

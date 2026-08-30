pub(crate) fn resolve(label_id: &str, _language: &str) -> Option<String> {
    let _ = label_id;
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_none_for_unknown_id() {
        assert!(resolve("unknown", "en").is_none());
        assert!(resolve("gba.system.unknown", "ja").is_none());
    }
}

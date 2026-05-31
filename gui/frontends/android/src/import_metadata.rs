#![cfg_attr(not(target_os = "android"), allow(dead_code))]

use std::path::Path;

const DEFAULT_IMPORTED_ROM_NAME: &str = "Imported ROM";

pub(crate) fn infer_import_metadata(
    display_name_hint: Option<&str>,
    uri: &str,
) -> (String, String) {
    let uri_metadata = metadata_from_candidate(uri_tail_candidate(uri));
    let display_metadata = display_name_hint.and_then(metadata_from_candidate);

    let display_name = display_metadata
        .as_ref()
        .map(|metadata| metadata.display_name.clone())
        .or_else(|| {
            uri_metadata
                .as_ref()
                .map(|metadata| metadata.display_name.clone())
        })
        .unwrap_or_else(|| DEFAULT_IMPORTED_ROM_NAME.to_string());
    let extension = display_metadata
        .as_ref()
        .filter(|metadata| !metadata.extension.is_empty())
        .map(|metadata| metadata.extension.clone())
        .or_else(|| {
            uri_metadata
                .as_ref()
                .filter(|metadata| !metadata.extension.is_empty())
                .map(|metadata| metadata.extension.clone())
        })
        .unwrap_or_default();

    (display_name, extension)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ImportMetadata {
    display_name: String,
    extension: String,
}

fn metadata_from_candidate(candidate: &str) -> Option<ImportMetadata> {
    let sanitized = sanitize_candidate(candidate);
    if sanitized.is_empty() {
        return None;
    }

    let path = Path::new(&sanitized);
    let display_name = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .unwrap_or(DEFAULT_IMPORTED_ROM_NAME)
        .to_string();
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_string();

    Some(ImportMetadata {
        display_name,
        extension,
    })
}

fn uri_tail_candidate(uri: &str) -> &str {
    uri.rsplit('/').next().unwrap_or(uri)
}

fn sanitize_candidate(candidate: &str) -> String {
    candidate
        .split(['?', '#'])
        .next()
        .unwrap_or(candidate)
        .trim()
        .replace("%20", " ")
}

#[cfg(test)]
mod tests {
    use super::infer_import_metadata;

    #[test]
    fn prefers_document_display_name_over_uri_tail() {
        assert_eq!(
            infer_import_metadata(
                Some("Super Mario Bros. (World).NES"),
                "content://documents/tree/primary%3ADownload/document/primary%3ADownload%2Ftmp.bin",
            ),
            ("Super Mario Bros. (World)".to_string(), "NES".to_string())
        );
    }

    #[test]
    fn falls_back_to_uri_tail_when_display_name_is_missing() {
        assert_eq!(
            infer_import_metadata(None, "content://provider/roms/Mega%20Man%202.zip?query=1"),
            ("Mega Man 2".to_string(), "zip".to_string())
        );
    }

    #[test]
    fn keeps_display_name_and_uses_uri_extension_when_needed() {
        assert_eq!(
            infer_import_metadata(
                Some("Metroid Save Slot"),
                "content://provider/roms/metroid.nes",
            ),
            ("Metroid Save Slot".to_string(), "nes".to_string())
        );
    }

    #[test]
    fn defaults_when_all_candidates_are_empty() {
        assert_eq!(
            infer_import_metadata(Some("   "), "content://provider/"),
            ("Imported ROM".to_string(), String::new())
        );
    }
}

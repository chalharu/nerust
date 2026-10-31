//! Directory metadata only: never open ROM contents, recurse or follow symlinks.
use std::{collections::BTreeMap, fs, path::PathBuf};

pub const MAX_RESULTS: usize = 256;
pub const MAX_VISITED: usize = 8192;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub path: PathBuf,
    pub name: String,
    pub system: String,
    pub bytes: u64,
}
impl Entry {
    pub fn key(&self) -> String {
        // Debug escaping distinguishes non-UTF8 paths (unlike lossy display).
        use std::fmt::Write;
        let mut key = String::new();
        for byte in format!("{:?}", self.path).bytes() {
            write!(&mut key, "{byte:02x}").expect("writing a String cannot fail");
        }
        key
    }
    pub fn label(&self) -> String {
        format!("{} | {} | {} bytes", self.name, self.system, self.bytes)
    }
    pub fn details(&self) -> String {
        format!(
            "{}\n{} | {} bytes | metadata only",
            self.path.display(),
            self.system,
            self.bytes
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Report {
    pub entries: Vec<Entry>,
    pub capped: bool,
}

pub fn scan(folder: PathBuf, filter: String) -> Result<Report, String> {
    let directory = fs::read_dir(&folder)
        .map_err(|error| format!("Cannot scan {}: {error}", folder.display()))?;
    let filter = filter.to_lowercase();
    let mut report = Report::default();
    let mut selected = BTreeMap::new();
    for (visited, entry) in directory.enumerate() {
        if visited >= MAX_VISITED {
            report.capped = true;
            break;
        }
        let entry = entry.map_err(|error| format!("Directory entry: {error}"))?;
        let path = entry.path();
        let system = match path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("nes") => "NES",
            Some("gb") => "GB",
            Some("gbc") => "GBC",
            Some("gba") => "GBA",
            _ => continue,
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.to_lowercase().contains(&filter) {
            continue;
        }
        let kind = entry
            .file_type()
            .map_err(|error| format!("File type: {error}"))?;
        if !kind.is_file() {
            continue;
        }
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("Metadata {}: {error}", path.display()))?;
        if !metadata.is_file() {
            continue;
        }
        selected.insert(
            path.clone(),
            Entry {
                path,
                name,
                system: system.into(),
                bytes: metadata.len(),
            },
        );
        if selected.len() > MAX_RESULTS {
            selected.pop_last();
            report.capped = true;
        }
    }
    report.entries = selected.into_values().collect();
    Ok(report)
}

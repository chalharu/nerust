//! Strict, versioned JSON and same-directory atomic publication.
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub schema_version: u32,
    pub name: String,
    pub volume: u8,
    pub scale: f64,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            schema_version: 1,
            name: "Player".into(),
            volume: 70,
            scale: 1.0,
        }
    }
}
impl Config {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 {
            return Err(format!(
                "unsupported schema_version: {} (expected 1)",
                self.schema_version
            ));
        }
        if self.name.trim().is_empty() || self.name.chars().count() > 80 {
            return Err("Name is required (1–80 characters)".into());
        }
        if self.volume > 100 {
            return Err("Volume must be an integer from 0 to 100".into());
        }
        if !self.scale.is_finite() || !(0.5..=3.0).contains(&self.scale) {
            return Err("Scale must be a finite number from 0.5 to 3".into());
        }
        Ok(())
    }
}

pub fn load(path: &Path) -> Result<Config, String> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Config::default()),
        Err(error) => return Err(format!("load {}: {error}", path.display())),
    };
    let config: Config = serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid config {}: {error}", path.display()))?;
    config.validate()?;
    Ok(config)
}

static NEXT: AtomicU64 = AtomicU64::new(0);
pub fn save(path: &Path, config: &Config) -> Result<(), String> {
    // Include the complete target filename, not just its extension. Never open
    // another process's temporary file, and never unlink a failed create_new.
    let mut name = path
        .file_name()
        .ok_or("config path has no filename")?
        .to_os_string();
    name.push(format!(
        ".{}.{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    save_via(path, &path.with_file_name(name), config)
}

struct OwnedTemp(Option<PathBuf>);
impl Drop for OwnedTemp {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = fs::remove_file(path);
        }
    }
}
fn save_via(path: &Path, temp: &Path, config: &Config) -> Result<(), String> {
    config.validate()?;
    let mut bytes = serde_json::to_vec_pretty(config).map_err(|error| error.to_string())?;
    bytes.push(b'\n');
    let result = (|| -> io::Result<()> {
        let mut file = OpenOptions::new().write(true).create_new(true).open(temp)?;
        let mut owned = OwnedTemp(Some(temp.to_owned()));
        let written = file.write_all(&bytes).and_then(|_| file.sync_all());
        // In particular on Windows, close the handle BEFORE cleanup on error.
        drop(file);
        written?;
        fs::rename(temp, path)?;
        // Never unlink this path again after publication (it could be reused).
        owned.0.take();
        Ok(())
    })();
    result.map_err(|error| format!(
        "save {}: {error} (atomic rename; replacement of an existing file may be unsupported on Windows)",
        path.display()
    ))
}

/// A unique, owned directory for tests/smoke. Existing directories are never adopted.
pub struct Sandbox {
    pub dir: PathBuf,
}
impl Sandbox {
    pub fn new() -> Result<Self, String> {
        let dir = std::env::temp_dir().join(format!(
            "wakui3-settings-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&dir).map_err(|error| error.to_string())?;
        Ok(Self { dir })
    }
    pub fn path(&self) -> PathBuf {
        self.dir.join("settings.json")
    }
}
impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn atomic_roundtrip_and_replacement() {
        let sandbox = Sandbox::new().unwrap();
        let path = sandbox.path();
        assert_eq!(load(&path).unwrap(), Config::default());
        save(&path, &Config::default()).unwrap();
        let changed = Config {
            name: "Ada".into(),
            ..Config::default()
        };
        #[cfg(not(windows))]
        {
            save(&path, &changed).unwrap();
            assert_eq!(load(&path).unwrap(), changed);
        }
        #[cfg(windows)]
        {
            let _ = changed;
        } // std::fs::rename replacement is platform-dependent.
        assert_eq!(fs::read_dir(&sandbox.dir).unwrap().count(), 1);
    }
    #[test]
    fn malformed_and_strict_schema_never_overwritten() {
        let sandbox = Sandbox::new().unwrap();
        for bytes in [
            "{",
            "{}",
            r#"{"schema_version":2,"name":"Ada","volume":1,"scale":1}"#,
            r#"{"schema_version":1,"name":"Ada","volume":1,"scale":1,"extra":0}"#,
            r#"{"schema_version":1,"name":"Ada","volume":1,"volume":2,"scale":1}"#,
            r#"{"schema_version":1,"name":"Ada","volume":101,"scale":1}"#,
            r#"{"schema_version":1,"name":"Ada","volume":"1","scale":1}"#,
        ] {
            fs::write(sandbox.path(), bytes).unwrap();
            assert!(load(&sandbox.path()).is_err());
            assert_eq!(fs::read_to_string(sandbox.path()).unwrap(), bytes);
        }
        assert!(load(&sandbox.dir).is_err());
    }
    #[test]
    fn wrong_temporary_is_not_deleted_or_overwritten() {
        let sandbox = Sandbox::new().unwrap();
        let temp = sandbox.dir.join("foreign.tmp");
        fs::write(&temp, "foreign").unwrap();
        fs::write(sandbox.path(), "original").unwrap();
        assert!(save_via(&sandbox.path(), &temp, &Config::default()).is_err());
        assert_eq!(fs::read_to_string(temp).unwrap(), "foreign");
        assert_eq!(fs::read_to_string(sandbox.path()).unwrap(), "original");
    }
    #[test]
    fn failure_preserves_original_and_cleans_only_owned_temp() {
        let sandbox = Sandbox::new().unwrap();
        save(&sandbox.path(), &Config::default()).unwrap();
        let original = fs::read(sandbox.path()).unwrap();
        let bad = Config {
            scale: f64::NAN,
            ..Config::default()
        };
        assert!(save(&sandbox.path(), &bad).is_err());
        assert_eq!(fs::read(sandbox.path()).unwrap(), original);
        // Rename into an occupied directory fails after write/sync.
        let target = sandbox.dir.join("occupied");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("keep"), "keep").unwrap();
        let temp = sandbox.dir.join("own.tmp");
        assert!(save_via(&target, &temp, &Config::default()).is_err());
        assert!(!temp.exists());
        assert_eq!(fs::read_to_string(target.join("keep")).unwrap(), "keep");
    }
}

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
    config.validate()?;
    let mut lock_name = path
        .file_name()
        .ok_or("config path has no filename")?
        .to_os_string();
    lock_name.push(".lock");
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path.with_file_name(lock_name))
        .map_err(|error| format!("save lock {}: {error}", path.display()))?;
    lock.try_lock()
        .map_err(|error| format!("another writer holds {}: {error}", path.display()))?;
    let _lock = WriterLock(lock);
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

struct WriterLock(fs::File);
impl Drop for WriterLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
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
    save_via_checked(path, temp, config, |_| Ok(()))
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum SaveStage {
    Create,
    Write,
    FileSync,
    Rename,
    #[cfg(unix)]
    DirectorySync,
}
fn save_via_checked(
    path: &Path,
    temp: &Path,
    config: &Config,
    check: impl Fn(SaveStage) -> io::Result<()>,
) -> Result<(), String> {
    config.validate()?;
    let mut bytes = serde_json::to_vec_pretty(config).map_err(|error| error.to_string())?;
    bytes.push(b'\n');
    let result = (|| -> io::Result<()> {
        check(SaveStage::Create)?;
        let mut file = OpenOptions::new().write(true).create_new(true).open(temp)?;
        let mut owned = OwnedTemp(Some(temp.to_owned()));
        let written = check(SaveStage::Write)
            .and_then(|_| file.write_all(&bytes))
            .and_then(|_| check(SaveStage::FileSync))
            .and_then(|_| file.sync_all());
        // In particular on Windows, close the handle BEFORE cleanup on error.
        drop(file);
        written?;
        check(SaveStage::Rename)?;
        fs::rename(temp, path)?;
        // Never unlink this path again after publication (it could be reused).
        owned.0.take();
        #[cfg(unix)]
        {
            check(SaveStage::DirectorySync)?;
            let parent = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            fs::File::open(parent)?.sync_all()?;
        }
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
    #[cfg(unix)]
    #[test]
    fn injected_save_failures_preserve_complete_json_and_cleanup_owned_temp() {
        for stage in [
            SaveStage::Create,
            SaveStage::Write,
            SaveStage::FileSync,
            SaveStage::Rename,
            SaveStage::DirectorySync,
        ] {
            for kind in [
                io::ErrorKind::PermissionDenied,
                io::ErrorKind::StorageFull,
                io::ErrorKind::Other,
            ] {
                let sandbox = Sandbox::new().unwrap();
                let path = sandbox.path();
                let temp = sandbox.dir.join("injected.tmp");
                save(&path, &Config::default()).unwrap();
                let changed = Config {
                    name: "New complete config".into(),
                    ..Config::default()
                };
                let result = save_via_checked(&path, &temp, &changed, |current| {
                    if current == stage {
                        Err(io::Error::new(kind, format!("injected {stage:?}")))
                    } else {
                        Ok(())
                    }
                });
                assert!(result.unwrap_err().contains("injected"));
                assert!(!temp.exists(), "{stage:?}/{kind:?}");
                assert_eq!(
                    load(&path).unwrap(),
                    if stage == SaveStage::DirectorySync {
                        changed
                    } else {
                        Config::default()
                    }
                );
                assert_eq!(fs::read_dir(&sandbox.dir).unwrap().count(), 2);
                save(&path, &Config::default()).unwrap();
                assert_eq!(load(&path).unwrap(), Config::default());
            }
        }
    }
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
        assert_eq!(fs::read_dir(&sandbox.dir).unwrap().count(), 2);
        assert!(sandbox.dir.join("settings.json.lock").exists());
    }
    #[test]
    fn concurrent_writer_is_rejected_without_changing_published_config() {
        let sandbox = Sandbox::new().unwrap();
        save(&sandbox.path(), &Config::default()).unwrap();
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .open(sandbox.dir.join("settings.json.lock"))
            .unwrap();
        lock.try_lock().unwrap();
        let lock = WriterLock(lock);
        let changed = Config {
            name: "other writer".into(),
            ..Config::default()
        };
        assert!(save(&sandbox.path(), &changed).is_err());
        assert_eq!(load(&sandbox.path()).unwrap(), Config::default());
        drop(lock);
        save(&sandbox.path(), &changed).unwrap();
        assert_eq!(load(&sandbox.path()).unwrap(), changed);
    }
    #[test]
    fn explicit_unlock_releases_inherited_file_descriptions() {
        let sandbox = Sandbox::new().unwrap();
        save(&sandbox.path(), &Config::default()).unwrap();
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .open(sandbox.dir.join("settings.json.lock"))
            .unwrap();
        lock.try_lock().unwrap();
        let inherited = lock.try_clone().unwrap();
        let lock = WriterLock(lock);
        drop(lock);
        save(&sandbox.path(), &Config::default()).unwrap();
        assert_eq!(load(&sandbox.path()).unwrap(), Config::default());
        drop(inherited);
    }
    #[cfg(unix)]
    #[test]
    fn writer_child() {
        if let Some(path) = std::env::var_os("WAKUI3_CRASH_PROBE") {
            use std::io::Write;
            let path = PathBuf::from(path);
            save(&path, &Config::default()).unwrap();
            println!("WRITER_READY");
            std::io::stdout().flush().unwrap();
            for value in 0..100_000 {
                save(
                    &path,
                    &Config {
                        name: format!("writer-{value}"),
                        ..Config::default()
                    },
                )
                .unwrap();
            }
        }
    }
    #[cfg(unix)]
    #[test]
    fn killed_writer_leaves_a_valid_old_or_new_config_and_releases_lock() {
        use std::{
            io::BufRead,
            process::{Command, Stdio},
        };
        let sandbox = Sandbox::new().unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "persistence::tests::writer_child", "--nocapture"])
            .env("WAKUI3_CRASH_PROBE", sandbox.path())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let output = child.stdout.take().unwrap();
        let mut ready = false;
        for line in io::BufReader::new(output).lines() {
            if line.unwrap().contains("WRITER_READY") {
                ready = true;
                break;
            }
        }
        let _ = child.kill();
        child.wait().unwrap();
        assert!(ready);
        let config = load(&sandbox.path()).unwrap();
        assert!(config == Config::default() || config.name.starts_with("writer-"));
        save(&sandbox.path(), &Config::default()).unwrap();
        assert_eq!(load(&sandbox.path()).unwrap(), Config::default());
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

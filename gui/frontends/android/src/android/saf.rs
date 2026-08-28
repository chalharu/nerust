use std::{
    io::Error as IoError,
    path::{Path, PathBuf},
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use jni::{
    JavaVM, jni_sig, jni_str,
    objects::{JObject, JString, JValue},
    refs::Global,
    sys::jobject,
};
use nerust_core_traits::identity::SystemIdentity;
use nerust_gui_runtime::settings::persistence::decode_document_tree_path;
use nerust_gui_shell::session::persistence::{
    AutoSaveBackend, FsSlotBackend, MapperSaveBackend, SlotBackend,
};
use nerust_persistence::{
    error::PersistenceError,
    model::{LoadedStateSlot, StateSlotSummary},
    slots::{build_state_slot_bytes, load_state_slot_bytes},
    thumbnail::ThumbnailSource,
};
use winit::platform::android::activity::AndroidApp;

#[derive(Clone)]
pub(crate) struct AndroidStorageBackend {
    app: AndroidApp,
}

impl AndroidStorageBackend {
    pub(crate) fn new(app: AndroidApp) -> Self {
        Self { app }
    }

    fn io_error(error: impl ToString) -> PersistenceError {
        PersistenceError::Io(IoError::other(error.to_string()))
    }

    fn decode(path: &Path) -> Option<(String, String)> {
        decode_document_tree_path(path)
    }

    fn validate_relative_path(path: &str) -> Result<(), PersistenceError> {
        if path.is_empty()
            || path.starts_with('/')
            || path.split('/').any(|segment| {
                segment.is_empty() || segment == "." || segment == ".." || segment.contains('\\')
            })
        {
            return Err(PersistenceError::Validation(
                "invalid Android SAF relative path".into(),
            ));
        }
        Ok(())
    }

    fn read(&self, uri: &str, relative: &str) -> Result<Option<Vec<u8>>, PersistenceError> {
        Self::validate_relative_path(relative)?;
        let value = self.call_string("readSafFile", uri, relative, None)?;
        value
            .map(|value| STANDARD.decode(value).map_err(Self::io_error))
            .transpose()
    }

    fn write(&self, uri: &str, relative: &str, bytes: &[u8]) -> Result<(), PersistenceError> {
        Self::validate_relative_path(relative)?;
        let encoded = STANDARD.encode(bytes);
        let result = self.call_string("writeSafFile", uri, relative, Some(&encoded))?;
        if result.as_deref() == Some("ok") {
            Ok(())
        } else {
            Err(Self::io_error("Android SAF write failed"))
        }
    }

    fn delete(&self, uri: &str, relative: &str) -> Result<(), PersistenceError> {
        Self::validate_relative_path(relative)?;
        let result = self.call_string("deleteSafFile", uri, relative, None)?;
        if matches!(result.as_deref(), Some("ok") | Some("missing")) {
            Ok(())
        } else {
            Err(Self::io_error("Android SAF delete failed"))
        }
    }

    fn list(&self, uri: &str, relative: &str) -> Result<Vec<String>, PersistenceError> {
        Self::validate_relative_path(relative)?;
        let value = self
            .call_string("listSafFiles", uri, relative, None)?
            .unwrap_or_else(|| "[]".to_string());
        serde_json::from_str(&value).map_err(Self::io_error)
    }

    fn call_string(
        &self,
        method: &str,
        uri: &str,
        relative: &str,
        payload: Option<&str>,
    ) -> Result<Option<String>, PersistenceError> {
        let vm = unsafe { JavaVM::from_raw(self.app.vm_as_ptr() as _) };
        vm.attach_current_thread(|env| {
            let activity_raw = self.app.activity_as_ptr() as jobject;
            let activity = unsafe { env.as_cast_raw::<Global<JObject<'static>>>(&activity_raw)? };
            let uri = JString::from_str(env, uri)?;
            let relative = JString::from_str(env, relative)?;
            let payload = payload
                .map(|value| JString::from_str(env, value))
                .transpose()?;
            let result = if let Some(payload) = payload.as_ref() {
                env.call_method(
                    activity.as_ref(),
                    jni_str!("writeSafFile"),
                    jni_sig!(
                        "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;"
                    ),
                    &[
                        JValue::Object(uri.as_ref()),
                        JValue::Object(relative.as_ref()),
                        JValue::Object(payload.as_ref()),
                    ],
                )?
            } else {
                let method = match method {
                    "readSafFile" => jni_str!("readSafFile"),
                    "deleteSafFile" => jni_str!("deleteSafFile"),
                    "listSafFiles" => jni_str!("listSafFiles"),
                    _ => return Err(jni::errors::Error::NullPtr("unknown SAF bridge method")),
                };
                env.call_method(
                    activity.as_ref(),
                    method,
                    jni_sig!("(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;"),
                    &[
                        JValue::Object(uri.as_ref()),
                        JValue::Object(relative.as_ref()),
                    ],
                )?
            };
            let object = result.l()?;
            if object.is_null() {
                Ok(None)
            } else {
                Ok(Some(JString::cast_local(env, object)?.try_to_string(env)?))
            }
        })
        .map_err(Self::io_error)
    }

    fn state_path(dir: &Path, slot_id: u64) -> PathBuf {
        dir.join(format!("{slot_id}.state"))
    }
}

impl SlotBackend for AndroidStorageBackend {
    fn scan(
        &self,
        dir: &Path,
        identity: &SystemIdentity,
    ) -> Result<Vec<StateSlotSummary>, PersistenceError> {
        let Some((uri, relative)) = Self::decode(dir) else {
            return FsSlotBackend.scan(dir, identity);
        };
        let mut result = Vec::new();
        for name in self
            .list(&uri, &relative)?
            .into_iter()
            .filter(|name| name.ends_with(".state"))
        {
            let path = dir.join(&name);
            let Some(bytes) = self.read(&uri, &format!("{relative}/{name}"))? else {
                continue;
            };
            if let Some(slot) = load_state_slot_bytes(path, bytes, Some(identity))? {
                result.push(slot.summary);
            }
        }
        result.sort_by_key(|slot| slot.slot_id);
        Ok(result)
    }

    fn allocate_next_id(&self, dir: &Path) -> Result<u64, PersistenceError> {
        let Some((uri, relative)) = Self::decode(dir) else {
            return FsSlotBackend.allocate_next_id(dir);
        };
        Ok(self
            .list(&uri, &relative)?
            .iter()
            .filter_map(|name| name.strip_suffix(".state")?.parse::<u64>().ok())
            .max()
            .unwrap_or(0)
            .saturating_add(1)
            .max(1))
    }

    fn write_slot(
        &self,
        dir: &Path,
        slot_id: u64,
        data: &[u8],
        identity: &SystemIdentity,
        thumbnail: Option<&ThumbnailSource>,
    ) -> Result<StateSlotSummary, PersistenceError> {
        let Some((uri, relative)) = Self::decode(dir) else {
            return FsSlotBackend.write_slot(dir, slot_id, data, identity, thumbnail);
        };
        let path = Self::state_path(dir, slot_id);
        let (bytes, summary) = build_state_slot_bytes(path, slot_id, data, identity, thumbnail)?;
        self.write(&uri, &format!("{relative}/{slot_id}.state"), &bytes)?;
        Ok(summary)
    }

    fn read_slot(
        &self,
        dir: &Path,
        slot_id: u64,
    ) -> Result<Option<LoadedStateSlot>, PersistenceError> {
        let Some((uri, relative)) = Self::decode(dir) else {
            return FsSlotBackend.read_slot(dir, slot_id);
        };
        let path = Self::state_path(dir, slot_id);
        let Some(bytes) = self.read(&uri, &format!("{relative}/{slot_id}.state"))? else {
            return Ok(None);
        };
        load_state_slot_bytes(path, bytes, None)
    }

    fn delete_slot(&self, dir: &Path, slot_id: u64) -> Result<(), PersistenceError> {
        let Some((uri, relative)) = Self::decode(dir) else {
            return FsSlotBackend.delete_slot(dir, slot_id);
        };
        self.delete(&uri, &format!("{relative}/{slot_id}.state"))
    }
}

impl AutoSaveBackend for AndroidStorageBackend {
    fn write_autosave(
        &self,
        dir: &Path,
        data: &[u8],
        identity: &SystemIdentity,
    ) -> Result<StateSlotSummary, PersistenceError> {
        let Some((uri, relative)) = Self::decode(dir) else {
            return FsSlotBackend.write_autosave(dir, data, identity);
        };
        let path = dir.join(".autosave_slot");
        let (bytes, summary) = build_state_slot_bytes(path, 0, data, identity, None)?;
        self.write(&uri, &format!("{relative}/.autosave_slot"), &bytes)?;
        Ok(summary)
    }

    fn read_autosave(
        &self,
        dir: &Path,
        identity: &SystemIdentity,
    ) -> Result<Option<LoadedStateSlot>, PersistenceError> {
        let Some((uri, relative)) = Self::decode(dir) else {
            return FsSlotBackend.read_autosave(dir, identity);
        };
        let path = dir.join(".autosave_slot");
        let Some(bytes) = self.read(&uri, &format!("{relative}/.autosave_slot"))? else {
            return Ok(None);
        };
        load_state_slot_bytes(path, bytes, Some(identity))
    }

    fn delete_autosave(&self, dir: &Path) -> Result<(), PersistenceError> {
        let Some((uri, relative)) = Self::decode(dir) else {
            return FsSlotBackend.delete_autosave(dir);
        };
        self.delete(&uri, &format!("{relative}/.autosave_slot"))
    }
}

impl MapperSaveBackend for AndroidStorageBackend {
    fn read_mapper_save(&self, path: &Path) -> Result<Option<Vec<u8>>, PersistenceError> {
        let Some((uri, relative)) = Self::decode(path) else {
            return FsSlotBackend.read_mapper_save(path);
        };
        self.read(&uri, &relative)
    }

    fn write_mapper_save(&self, path: &Path, data: &[u8]) -> Result<(), PersistenceError> {
        let Some((uri, relative)) = Self::decode(path) else {
            return FsSlotBackend.write_mapper_save(path, data);
        };
        self.write(&uri, &relative, data)
    }

    fn write_recovery_mapper_save(
        &self,
        path: &Path,
        data: &[u8],
    ) -> Result<PathBuf, PersistenceError> {
        let Some((uri, relative)) = Self::decode(path) else {
            return FsSlotBackend.write_recovery_mapper_save(path, data);
        };
        let recovery = format!("{relative}.recovery");
        self.write(&uri, &recovery, data)?;
        Ok(PathBuf::from(format!("{}.recovery", path.display())))
    }
}

#[cfg(test)]
mod tests {
    use super::AndroidStorageBackend;

    #[test]
    fn relative_path_validation_rejects_traversal_and_empty_segments() {
        assert!(AndroidStorageBackend::validate_relative_path("gbc/id/states").is_ok());
        for invalid in [
            "",
            "/root",
            "../save",
            "gbc//save",
            "gbc/./save",
            "gbc\\save",
        ] {
            assert!(AndroidStorageBackend::validate_relative_path(invalid).is_err());
        }
    }
}

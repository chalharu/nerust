use std::{
    io::Error as IoError,
    path::{Path, PathBuf},
};

use jni::{
    JavaVM,
    objects::{JObject, JString, JValue},
    refs::Global,
    sys::jobject,
};
use nerust_core_traits::identity::SystemIdentity;
use nerust_gui_runtime::settings::persistence::decode_document_tree_path;
use nerust_gui_shell::session::persistence::{
    AutoSaveBackend, FsAutosave, FsMapper, FsSlots, MapperSaveBackend, SlotBackend,
};
use nerust_persistence::{
    error::PersistenceError,
    model::{LoadedStateSlot, StateSlotSummary},
    slots::{build_state_slot_bytes, load_state_slot_bytes},
    thumbnail::ThumbnailSource,
};
use winit::platform::android::activity::AndroidApp;

use super::bridge;
use crate::{bridge_contract::SAF_CHUNK_BYTES, saf_decision::collect_stream};

#[derive(Clone)]
pub(super) struct AndroidStorageBackend {
    app: AndroidApp,
}

impl AndroidStorageBackend {
    pub(super) fn new(app: AndroidApp) -> Self {
        Self { app }
    }

    fn persist(error: impl std::fmt::Display) -> PersistenceError {
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

    /// One fixed raw stream: open -1 = missing, 0 = error, positive = session.
    /// Nonempty chunks are data, empty chunks EOF, null/exception an I/O error.
    fn read(&self, uri: &str, relative: &str) -> Result<Option<Vec<u8>>, PersistenceError> {
        Self::validate_relative_path(relative)?;
        let vm = unsafe { JavaVM::from_raw(self.app.vm_as_ptr() as _) };
        bridge::with_java_env(&vm, |env| {
            let activity_raw = self.app.activity_as_ptr() as jobject;
            let activity = unsafe { env.as_cast_raw::<Global<JObject<'static>>>(&activity_raw)? };
            let activity = activity.as_ref();
            let uri = JString::from_str(env, uri)?;
            let relative = JString::from_str(env, relative)?;
            let session = Self::call_open_read_session_inner(env, activity, &uri, &relative)?;
            if session == -1 {
                return Ok(None);
            }
            if session <= 0 {
                return Err(jni::errors::Error::NullPtr("Android SAF open failed"));
            }
            let mut result =
                collect_stream(|| Self::call_read_session_chunk_inner(env, activity, session));
            // Clear a failed read's pending exception before cleanup; retain
            // the original error so partial bytes never become success.
            if env.exception_check() {
                env.exception_clear();
                result = Err(result.err().unwrap_or(jni::errors::Error::JavaException));
            }
            let close_result = Self::call_close_read_session_inner(env, activity, session);
            let bytes = result?;
            close_result?;
            Ok::<_, jni::errors::Error>(Some(bytes))
        })
        .map_err(Self::persist)
    }

    /// Open a session: -1 means missing, 0 means error.
    fn call_open_read_session_inner(
        env: &mut jni::Env<'_>,
        activity: &JObject<'_>,
        uri: &JString<'_>,
        relative: &JString<'_>,
    ) -> jni::errors::Result<i64> {
        Ok(bridge::call_bridge_method(
            env,
            activity,
            &bridge::methods::OPEN_SAF_READ_SESSION,
            &[
                JValue::Object(uri.as_ref()),
                JValue::Object(relative.as_ref()),
            ],
        )?
        .j()?)
    }

    /// One sequential chunk from an open session; empty marks end of stream.
    fn call_read_session_chunk_inner(
        env: &mut jni::Env<'_>,
        activity: &JObject<'_>,
        session: i64,
    ) -> jni::errors::Result<Vec<u8>> {
        env.with_local_frame(4, |env| {
            let object = bridge::call_bridge_method(
                env,
                activity,
                &bridge::methods::READ_SAF_READ_SESSION,
                &[JValue::Long(session), JValue::Int(SAF_CHUNK_BYTES as i32)],
            )?
            .l()?;
            if object.is_null() {
                return Err(jni::errors::Error::NullPtr("Android SAF read failed"));
            }
            let array = env.cast_local::<jni::objects::JByteArray>(object)?;
            env.convert_byte_array(&array)
        })
    }

    /// Close a server-side read session; a missing session is a no-op.
    fn call_close_read_session_inner(
        env: &mut jni::Env<'_>,
        activity: &JObject<'_>,
        session: i64,
    ) -> jni::errors::Result<()> {
        bridge::call_bridge_method(
            env,
            activity,
            &bridge::methods::CLOSE_SAF_READ_SESSION,
            &[JValue::Long(session)],
        )?;
        Ok(())
    }

    fn write(&self, uri: &str, relative: &str, bytes: &[u8]) -> Result<(), PersistenceError> {
        Self::validate_relative_path(relative)?;
        let vm = unsafe { JavaVM::from_raw(self.app.vm_as_ptr() as _) };
        bridge::with_java_env(&vm, |env| {
            let activity_raw = self.app.activity_as_ptr() as jobject;
            let activity = unsafe { env.as_cast_raw::<Global<JObject<'static>>>(&activity_raw)? };
            let activity = activity.as_ref();
            let uri = JString::from_str(env, uri)?;
            let relative = JString::from_str(env, relative)?;
            // An empty write must still truncate/create the document.
            for (index, chunk) in bytes
                .chunks(SAF_CHUNK_BYTES)
                .enumerate()
                .chain(bytes.is_empty().then_some((0, &[][..])))
            {
                let result = Self::call_write_chunk_bytes_inner(
                    env,
                    activity,
                    &uri,
                    &relative,
                    chunk,
                    index > 0,
                )?;
                if !result {
                    return Err(jni::errors::Error::NullPtr("Android SAF write failed"));
                }
            }
            Ok::<(), jni::errors::Error>(())
        })
        .map_err(Self::persist)
    }

    /// One raw `byte[]` chunk write; `append` selects `"wa"` (append) over
    /// `"rwt"` (overwrite).
    fn call_write_chunk_bytes_inner(
        env: &mut jni::Env<'_>,
        activity: &JObject<'_>,
        uri: &JString<'_>,
        relative: &JString<'_>,
        chunk: &[u8],
        append: bool,
    ) -> jni::errors::Result<bool> {
        env.with_local_frame(4, |env| {
            let buffer = env.byte_array_from_slice(chunk)?;
            bridge::call_bridge_method(
                env,
                activity,
                &bridge::methods::WRITE_SAF_FILE_CHUNK_BYTES,
                &[
                    JValue::Object(uri.as_ref()),
                    JValue::Object(relative.as_ref()),
                    JValue::Object(buffer.as_ref()),
                    JValue::Bool(append),
                ],
            )?
            .z()
        })
    }

    fn delete(&self, uri: &str, relative: &str) -> Result<(), PersistenceError> {
        Self::validate_relative_path(relative)?;
        let result = self.call_saf_string(&bridge::methods::DELETE_SAF_FILE, uri, relative)?;
        if matches!(result.as_deref(), Some("ok") | Some("missing")) {
            Ok(())
        } else {
            Err(PersistenceError::Io(IoError::other(
                "Android SAF delete failed",
            )))
        }
    }

    /// One listing contract: negative sizes mean unknown, never trigger fallback.
    fn sized_listing(
        &self,
        uri: &str,
        relative: &str,
    ) -> Result<Vec<(String, Option<i64>)>, PersistenceError> {
        let value = self
            .call_saf_string(&bridge::methods::LIST_SAF_FILES_WITH_SIZES, uri, relative)?
            .ok_or_else(|| Self::persist("Android SAF listing failed"))?;
        let entries: Vec<SizedEntry> = serde_json::from_str(&value).map_err(Self::persist)?;
        Ok(entries
            .into_iter()
            .map(|entry| {
                let size = entry.known_size();
                (entry.name, size)
            })
            .collect())
    }

    fn call_saf_string(
        &self,
        method: &bridge::methods::BridgeMethod,
        uri: &str,
        relative: &str,
    ) -> Result<Option<String>, PersistenceError> {
        let vm = unsafe { JavaVM::from_raw(self.app.vm_as_ptr() as _) };
        bridge::with_java_env(&vm, |env| {
            let activity_raw = self.app.activity_as_ptr() as jobject;
            let activity = unsafe { env.as_cast_raw::<Global<JObject<'static>>>(&activity_raw)? };
            let activity = activity.as_ref();
            let uri = JString::from_str(env, uri)?;
            let relative = JString::from_str(env, relative)?;
            let args = [
                JValue::Object(uri.as_ref()),
                JValue::Object(relative.as_ref()),
            ];
            let object = bridge::call_bridge_method(env, activity, method, &args)?.l()?;
            if object.is_null() {
                Ok(None)
            } else {
                Ok(Some(JString::cast_local(env, object)?.try_to_string(env)?))
            }
        })
        .map_err(Self::persist)
    }

    fn state_path(dir: &Path, slot_id: u64) -> PathBuf {
        dir.join(format!("{slot_id}.state"))
    }
}

/// One entry of the `listSafFilesWithSizes` batch; unknown sizes are `None`.
#[derive(Debug, serde::Deserialize)]
struct SizedEntry {
    name: String,
    #[serde(default)]
    size: Option<i64>,
}

impl SizedEntry {
    /// Negative Kotlin lengths normalize to unknown.
    fn known_size(&self) -> Option<i64> {
        self.size.filter(|size| *size >= 0)
    }
}

impl SlotBackend for AndroidStorageBackend {
    /// SAF scan runs off the event loop; one crossing carries every (name, size).
    fn scan(
        &self,
        dir: &Path,
        identity: &SystemIdentity,
    ) -> Result<Vec<StateSlotSummary>, PersistenceError> {
        let Some((uri, relative)) = Self::decode(dir) else {
            return FsSlots.scan(dir, identity);
        };
        let entries: Vec<(String, Option<i64>)> = self.sized_listing(&uri, &relative)?;
        let mut result = Vec::new();
        for (name, size) in entries
            .into_iter()
            .filter(|(name, _)| name.ends_with(".state"))
        {
            // Empty files cannot hold an archive; unknown sizes still read.
            if size == Some(0) {
                continue;
            }
            let path = dir.join(&name);
            let child = format!("{relative}/{name}");
            let bytes = self.read(&uri, &child)?;
            let Some(bytes) = bytes else {
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
            return FsSlots.allocate_next_id(dir);
        };
        Ok(self
            .sized_listing(&uri, &relative)?
            .iter()
            .filter_map(|(name, _)| name.strip_suffix(".state")?.parse::<u64>().ok())
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
            return FsSlots.write_slot(dir, slot_id, data, identity, thumbnail);
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
            return FsSlots.read_slot(dir, slot_id);
        };
        let path = Self::state_path(dir, slot_id);
        let Some(bytes) = self.read(&uri, &format!("{relative}/{slot_id}.state"))? else {
            return Ok(None);
        };
        load_state_slot_bytes(path, bytes, None)
    }

    fn delete_slot(&self, dir: &Path, slot_id: u64) -> Result<(), PersistenceError> {
        let Some((uri, relative)) = Self::decode(dir) else {
            return FsSlots.delete_slot(dir, slot_id);
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
            return FsAutosave.write_autosave(dir, data, identity);
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
            return FsAutosave.read_autosave(dir, identity);
        };
        let path = dir.join(".autosave_slot");
        let Some(bytes) = self.read(&uri, &format!("{relative}/.autosave_slot"))? else {
            return Ok(None);
        };
        load_state_slot_bytes(path, bytes, Some(identity))
    }

    fn delete_autosave(&self, dir: &Path) -> Result<(), PersistenceError> {
        let Some((uri, relative)) = Self::decode(dir) else {
            return FsAutosave.delete_autosave(dir);
        };
        self.delete(&uri, &format!("{relative}/.autosave_slot"))
    }
}

impl MapperSaveBackend for AndroidStorageBackend {
    fn read_mapper_save(&self, path: &Path) -> Result<Option<Vec<u8>>, PersistenceError> {
        let Some((uri, relative)) = Self::decode(path) else {
            return FsMapper.read_mapper_save(path);
        };
        self.read(&uri, &relative)
    }

    fn write_mapper_save(&self, path: &Path, data: &[u8]) -> Result<(), PersistenceError> {
        let Some((uri, relative)) = Self::decode(path) else {
            return FsMapper.write_mapper_save(path, data);
        };
        self.write(&uri, &relative, data)
    }

    fn write_recovery_mapper_save(
        &self,
        path: &Path,
        data: &[u8],
    ) -> Result<PathBuf, PersistenceError> {
        let Some((uri, relative)) = Self::decode(path) else {
            return FsMapper.write_recovery_mapper_save(path, data);
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

    /// Pin shared with the Kotlin side: both crossings assume 64KB.
    /// Change both together; this test fails loudly on drift.
    #[test]
    fn saf_chunk_bytes_matches_kotlin_contract() {
        assert_eq!(super::SAF_CHUNK_BYTES, 64 * 1024);
    }

    #[test]
    fn sized_entry_batch_normalizes_names_and_sizes() {
        let entries: Vec<super::SizedEntry> = serde_json::from_str(
            r#"[{"name":"0.state","size":42},{"name":"1.state"},{"name":"2.state","size":-1}]"#,
        )
        .unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].known_size(), Some(42));
        // Missing sizes are unknown, never a decode failure.
        assert_eq!(entries[1].known_size(), None);
        // Negative Kotlin lengths normalize to unknown.
        assert_eq!(entries[2].known_size(), None);
    }
}

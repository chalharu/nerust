use std::io::Error as IoError;

use jni::{
    JavaVM,
    objects::{JObject, JString, JValue},
    refs::Global,
    sys::jobject,
};
use nerust_core_traits::identity::SystemIdentity;
use nerust_gui_shell::session::persistence::{
    AutoSaveBackend, FsAutosave, FsMapper, FsSlots, MapperSaveBackend, SlotBackend,
};
use nerust_persistence::{
    error::PersistenceError,
    model::{LoadedStateSlot, StateSlotSummary},
    sidecar::StorageLocation,
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

    /// Only the JNI boundary projects the typed document key to bridge strings.
    fn document_key(location: &StorageLocation) -> Result<(&str, &str), PersistenceError> {
        match location {
            StorageLocation::DocumentTree { uri, relative } => Ok((uri, relative.as_str())),
            StorageLocation::Native(_) => Err(PersistenceError::Validation(
                "native storage passed to SAF".into(),
            )),
        }
    }

    /// One fixed raw stream: open -1 = missing, 0 = error, positive = session.
    /// Nonempty chunks are data, empty chunks EOF, null/exception an I/O error.
    fn read(&self, location: &StorageLocation) -> Result<Option<Vec<u8>>, PersistenceError> {
        let (uri, relative) = Self::document_key(location)?;
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

    fn write(&self, location: &StorageLocation, bytes: &[u8]) -> Result<(), PersistenceError> {
        let (uri, relative) = Self::document_key(location)?;
        let vm = unsafe { JavaVM::from_raw(self.app.vm_as_ptr() as _) };
        bridge::with_java_env(&vm, |env| {
            let activity_raw = self.app.activity_as_ptr() as jobject;
            let activity = unsafe { env.as_cast_raw::<Global<JObject<'static>>>(&activity_raw)? };
            let activity = activity.as_ref();
            let uri = JString::from_str(env, uri)?;
            let relative = JString::from_str(env, relative)?;
            let session = bridge::call_bridge_method(
                env,
                activity,
                &bridge::methods::OPEN_SAF_WRITE_SESSION,
                &[
                    JValue::Object(uri.as_ref()),
                    JValue::Object(relative.as_ref()),
                ],
            )?
            .j()?;
            if session <= 0 {
                return Err(jni::errors::Error::NullPtr("Android SAF staging failed"));
            }
            let result = (|| {
                for chunk in bytes.chunks(SAF_CHUNK_BYTES) {
                    if !Self::call_write_chunk_bytes_inner(env, activity, session, chunk)? {
                        return Err(jni::errors::Error::NullPtr("Android SAF write failed"));
                    }
                }
                Ok(())
            })();
            let result = if env.exception_check() {
                env.exception_clear();
                Err(result.err().unwrap_or(jni::errors::Error::JavaException))
            } else {
                result
            };
            let finish = bridge::call_bridge_method(
                env,
                activity,
                &bridge::methods::CLOSE_SAF_WRITE_SESSION,
                &[JValue::Long(session), JValue::Bool(result.is_ok())],
            );
            result?;
            if !finish?.z()? {
                return Err(jni::errors::Error::NullPtr("Android SAF commit failed"));
            }
            Ok::<(), jni::errors::Error>(())
        })
        .map_err(Self::persist)
    }

    /// One raw `byte[]` chunk on the existing staged output stream.
    fn call_write_chunk_bytes_inner(
        env: &mut jni::Env<'_>,
        activity: &JObject<'_>,
        session: i64,
        chunk: &[u8],
    ) -> jni::errors::Result<bool> {
        env.with_local_frame(4, |env| {
            let buffer = env.byte_array_from_slice(chunk)?;
            bridge::call_bridge_method(
                env,
                activity,
                &bridge::methods::WRITE_SAF_WRITE_SESSION,
                &[JValue::Long(session), JValue::Object(buffer.as_ref())],
            )?
            .z()
        })
    }

    fn delete(&self, location: &StorageLocation) -> Result<(), PersistenceError> {
        let (uri, relative) = Self::document_key(location)?;
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
        location: &StorageLocation,
    ) -> Result<Vec<(String, Option<i64>)>, PersistenceError> {
        let (uri, relative) = Self::document_key(location)?;
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

    fn state_path(
        dir: &StorageLocation,
        slot_id: u64,
    ) -> Result<StorageLocation, PersistenceError> {
        dir.child(&format!("{slot_id}.state"))
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
        dir: &StorageLocation,
        identity: &SystemIdentity,
    ) -> Result<Vec<StateSlotSummary>, PersistenceError> {
        if let StorageLocation::Native(_) = dir {
            return FsSlots.scan(dir, identity);
        }
        let entries: Vec<(String, Option<i64>)> = self.sized_listing(dir)?;
        let mut result = Vec::new();
        for (name, size) in entries
            .into_iter()
            .filter(|(name, _)| name.ends_with(".state"))
        {
            // Empty files cannot hold an archive; unknown sizes still read.
            if size == Some(0) {
                continue;
            }
            let path = dir.child(&name)?;
            let bytes = self.read(&path)?;
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

    fn allocate_next_id(&self, dir: &StorageLocation) -> Result<u64, PersistenceError> {
        if let StorageLocation::Native(_) = dir {
            return FsSlots.allocate_next_id(dir);
        }
        Ok(self
            .sized_listing(dir)?
            .iter()
            .filter_map(|(name, _)| name.strip_suffix(".state")?.parse::<u64>().ok())
            .max()
            .unwrap_or(0)
            .saturating_add(1)
            .max(1))
    }

    fn write_slot(
        &self,
        dir: &StorageLocation,
        slot_id: u64,
        data: &[u8],
        identity: &SystemIdentity,
        thumbnail: Option<&ThumbnailSource>,
    ) -> Result<StateSlotSummary, PersistenceError> {
        if let StorageLocation::Native(_) = dir {
            return FsSlots.write_slot(dir, slot_id, data, identity, thumbnail);
        }
        let path = Self::state_path(dir, slot_id)?;
        let (bytes, summary) =
            build_state_slot_bytes(path.clone(), slot_id, data, identity, thumbnail)?;
        self.write(&path, &bytes)?;
        Ok(summary)
    }

    fn read_slot(
        &self,
        dir: &StorageLocation,
        slot_id: u64,
    ) -> Result<Option<LoadedStateSlot>, PersistenceError> {
        if let StorageLocation::Native(_) = dir {
            return FsSlots.read_slot(dir, slot_id);
        }
        let path = Self::state_path(dir, slot_id)?;
        let Some(bytes) = self.read(&path)? else {
            return Ok(None);
        };
        load_state_slot_bytes(path, bytes, None)
    }

    fn delete_slot(&self, dir: &StorageLocation, slot_id: u64) -> Result<(), PersistenceError> {
        if let StorageLocation::Native(_) = dir {
            return FsSlots.delete_slot(dir, slot_id);
        }
        self.delete(&Self::state_path(dir, slot_id)?)
    }
}

impl AutoSaveBackend for AndroidStorageBackend {
    fn write_autosave(
        &self,
        dir: &StorageLocation,
        data: &[u8],
        identity: &SystemIdentity,
    ) -> Result<StateSlotSummary, PersistenceError> {
        if let StorageLocation::Native(_) = dir {
            return FsAutosave.write_autosave(dir, data, identity);
        }
        let path = dir.child(".autosave_slot")?;
        let (bytes, summary) = build_state_slot_bytes(path.clone(), 0, data, identity, None)?;
        self.write(&path, &bytes)?;
        Ok(summary)
    }

    fn read_autosave(
        &self,
        dir: &StorageLocation,
        identity: &SystemIdentity,
    ) -> Result<Option<LoadedStateSlot>, PersistenceError> {
        if let StorageLocation::Native(_) = dir {
            return FsAutosave.read_autosave(dir, identity);
        }
        let path = dir.child(".autosave_slot")?;
        let Some(bytes) = self.read(&path)? else {
            return Ok(None);
        };
        load_state_slot_bytes(path, bytes, Some(identity))
    }

    fn delete_autosave(&self, dir: &StorageLocation) -> Result<(), PersistenceError> {
        if let StorageLocation::Native(_) = dir {
            return FsAutosave.delete_autosave(dir);
        }
        self.delete(&dir.child(".autosave_slot")?)
    }
}

impl MapperSaveBackend for AndroidStorageBackend {
    fn read_mapper_save(
        &self,
        path: &StorageLocation,
    ) -> Result<Option<Vec<u8>>, PersistenceError> {
        if let StorageLocation::Native(_) = path {
            return FsMapper.read_mapper_save(path);
        }
        self.read(path)
    }

    fn write_mapper_save(
        &self,
        path: &StorageLocation,
        data: &[u8],
    ) -> Result<(), PersistenceError> {
        if let StorageLocation::Native(_) = path {
            return FsMapper.write_mapper_save(path, data);
        }
        self.write(path, data)
    }

    fn write_recovery_mapper_save(
        &self,
        path: &StorageLocation,
        data: &[u8],
    ) -> Result<StorageLocation, PersistenceError> {
        let StorageLocation::DocumentTree { uri, relative } = path else {
            return FsMapper.write_recovery_mapper_save(path, data);
        };
        let recovery =
            StorageLocation::document_tree(uri, format!("{}.recovery", relative.as_str()))?;
        self.write(&recovery, data)?;
        Ok(recovery)
    }
}

#[cfg(test)]
mod tests {
    use super::AndroidStorageBackend;

    #[test]
    fn document_keys_reach_the_bridge_without_path_encoding() {
        let uri = "content://provider/tree/primary%3ASaves";
        let dir = super::StorageLocation::document_tree(uri, "gbc/id/states").unwrap();
        let path = AndroidStorageBackend::state_path(&dir, 7).unwrap();
        assert_eq!(
            AndroidStorageBackend::document_key(&path).unwrap(),
            (uri, "gbc/id/states/7.state")
        );
        assert!(
            AndroidStorageBackend::document_key(&super::StorageLocation::from("native.sav"))
                .is_err()
        );
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

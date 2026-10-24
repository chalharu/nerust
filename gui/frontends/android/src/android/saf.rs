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
    slots::{
        MAX_SLOT_ARCHIVE_BYTES, build_state_slot_bytes, load_state_slot_bytes,
        load_state_summary_bytes, load_state_summary_from_metadata_bytes,
        publish_state_with_sidecar, quarantine_by_copy,
    },
    thumbnail::ThumbnailSource,
};
use winit::platform::android::activity::AndroidApp;

use super::bridge;
use crate::{
    bridge_contract::{SAF_CHUNK_BYTES, SAF_SUMMARY_BYTES},
    saf_decision::{CollectError, collect_stream_capped},
};

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
            StorageLocation::DocumentTree { uri, relative } => {
                Ok((uri.as_str(), relative.as_str()))
            }
            StorageLocation::Native(_) => Err(PersistenceError::Validation(
                "native storage passed to SAF".into(),
            )),
        }
    }

    /// One fixed raw stream: open -1 = missing, 0 = error, positive = session.
    /// Nonempty chunks are data, empty chunks EOF, null/exception an I/O error.
    fn read(&self, location: &StorageLocation) -> Result<Option<Vec<u8>>, PersistenceError> {
        let limit = usize::try_from(MAX_SLOT_ARCHIVE_BYTES).unwrap_or(usize::MAX);
        self.read_capped(location, limit)
    }

    /// Bounded variant: sidecars use the metadata cap so a huge summary stops mid-transfer.
    fn read_capped(
        &self,
        location: &StorageLocation,
        limit: usize,
    ) -> Result<Option<Vec<u8>>, PersistenceError> {
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
            let mut result = collect_stream_capped(
                || Self::call_read_session_chunk_inner(env, activity, session),
                limit,
            )
            .map_err(|error| match error {
                CollectError::Read(error) => error,
                CollectError::OverLimit { .. } => {
                    jni::errors::Error::NullPtr("Android SAF document exceeds scan bound")
                }
            });
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
        bridge::call_bridge_method(
            env,
            activity,
            &bridge::methods::OPEN_SAF_READ_SESSION,
            &[
                JValue::Object(uri.as_ref()),
                JValue::Object(relative.as_ref()),
            ],
        )?
        .j()
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
    /// Entries also carry sidecar bytes when the provider listed them.
    fn sized_listing(
        &self,
        location: &StorageLocation,
    ) -> Result<Vec<SizedEntry>, PersistenceError> {
        let (uri, relative) = Self::document_key(location)?;
        let value = self
            .call_saf_string(&bridge::methods::LIST_SAF_FILES_WITH_SIZES, uri, relative)?
            .ok_or_else(|| Self::persist("Android SAF listing failed"))?;
        let entries: Vec<SizedEntry> = serde_json::from_str(&value).map_err(Self::persist)?;
        Ok(entries)
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

    /// Scan transfer guard: known-oversized files cannot load, so scans skip early.
    fn scan_candidate(size: Option<i64>) -> bool {
        size.is_none_or(|size| (size as u64) <= MAX_SLOT_ARCHIVE_BYTES)
    }

    fn state_path(
        dir: &StorageLocation,
        slot_id: u64,
    ) -> Result<StorageLocation, PersistenceError> {
        dir.child(&format!("{slot_id}.state"))
    }

    /// Bounded sidecar lookup for one archive: metadata bytes with fallback to the archive read.
    /// Transfer and decode caps match the wire pin; the decode layer rechecks.
    fn read_summary_sidecar(
        &self,
        dir: &StorageLocation,
        state_name: &str,
        identity: &SystemIdentity,
    ) -> Option<StateSlotSummary> {
        let base = state_name.strip_suffix(".state")?;
        let slot_id = base.parse::<u64>().ok()?;
        let path = dir.child(&format!("{base}.summary")).ok()?;
        let bytes = self.read_capped(&path, SAF_SUMMARY_BYTES).ok()??;
        load_state_summary_from_metadata_bytes(path, &bytes, Some(identity))
            .ok()?
            .filter(|summary| summary.slot_id == slot_id)
    }
}

/// One entry of the `listSafFilesWithSizes` batch; unknown sizes are `None`.
/// `summary` carries scan sidecar bytes when the provider listed them.
#[derive(Debug, serde::Deserialize)]
struct SizedEntry {
    name: String,
    #[serde(default)]
    size: Option<i64>,
    #[serde(default)]
    summary: Option<String>,
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
        let entries = self.sized_listing(dir)?;
        let mut result = Vec::new();
        for entry in entries
            .into_iter()
            .filter(|entry| entry.name.ends_with(".state"))
        {
            let size = entry.known_size();
            // Untrusted listing names never abort the scan: skip and move on.
            let path = match dir.child(&entry.name) {
                Ok(path) => path,
                Err(_) => {
                    log::warn!("SAF scan: skipping invalid entry name");
                    continue;
                }
            };
            // Empty files cannot hold an archive; unknown sizes still read.
            // Oversized files cannot load, so scans skip before transfer.
            if size == Some(0) || !Self::scan_candidate(size) {
                continue;
            }
            // Batched sidecar first: one pass serves the listing, then fallback
            // to the single sidecar read and the archive.
            let entry_id = entry
                .name
                .strip_suffix(".state")
                .and_then(|base| base.parse::<u64>().ok());
            let batched: Option<StateSlotSummary> = entry
                .summary
                .as_deref()
                .and_then(|encoded| {
                    use base64::Engine as _;
                    base64::engine::general_purpose::STANDARD
                        .decode(encoded)
                        .ok()
                })
                .filter(|bytes| !bytes.is_empty())
                .and_then(|bytes| {
                    load_state_summary_from_metadata_bytes(path.clone(), &bytes, Some(identity))
                        .ok()
                })
                .flatten()
                // A sidecar names its slot: a mismatched id falls through to the
                // single sidecar read and then the archive.
                .filter(|summary| Some(summary.slot_id) == entry_id);
            if let Some(summary) = batched {
                result.push(summary);
                continue;
            }
            // Sidecar first: one bounded metadata transfer instead of the
            // whole archive. Any miss or mismatch falls back to the archive.
            if let Some(summary) = Self::read_summary_sidecar(self, dir, &entry.name, identity) {
                result.push(summary);
                continue;
            }
            let bytes = match self.read(&path) {
                Ok(bytes) => bytes,
                // One bad slot must not fail the whole scan.
                Err(error) => {
                    log::warn!("SAF scan: skipping unreadable entry: {error:?}");
                    continue;
                }
            };
            let Some(bytes) = bytes else {
                continue;
            };
            match load_state_summary_bytes(path, bytes, Some(identity)) {
                Ok(Some(summary)) => result.push(summary),
                Ok(None) => {}
                Err(error) => {
                    log::warn!("SAF scan: skipping corrupt entry: {error:?}");
                }
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
            .filter_map(|entry| entry.name.strip_suffix(".state")?.parse::<u64>().ok())
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
        let (bytes, metadata_bytes, summary) =
            build_state_slot_bytes(path.clone(), slot_id, data, identity, thumbnail)?;
        // Same-generation publish: the scan never adopts a stale summary.
        let sidecar = dir.child(&format!("{slot_id}.summary")).ok();
        publish_state_with_sidecar(
            || {
                sidecar
                    .as_ref()
                    .map(|sidecar| self.delete(sidecar))
                    .unwrap_or(Ok(()))
            },
            || self.write(&path, &bytes),
            || {
                sidecar
                    .as_ref()
                    .map(|sidecar| self.write(sidecar, &metadata_bytes))
                    .unwrap_or(Ok(()))
            },
        )?;
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
        self.delete(&Self::state_path(dir, slot_id)?)?;
        // An orphaned sidecar is never read without its archive, so its
        // removal stays best-effort.
        let _ = dir
            .child(&format!("{slot_id}.summary"))
            .and_then(|path| self.delete(&path));
        Ok(())
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
        let (bytes, _, summary) = build_state_slot_bytes(path.clone(), 0, data, identity, None)?;
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

    /// Quarantine by copy-then-delete: the original is removed only after the copy is stored.
    fn quarantine_autosave(
        &self,
        dir: &StorageLocation,
    ) -> Result<Option<StorageLocation>, PersistenceError> {
        if let StorageLocation::Native(_) = dir {
            return FsAutosave.quarantine_autosave(dir);
        }
        let path = dir.child(".autosave_slot")?;
        let quarantined = dir.child(".autosave_slot.quarantined")?;
        let quarantine_target = quarantined.clone();
        quarantine_by_copy(
            || self.read(&path),
            |bytes| self.write(&quarantine_target, bytes),
            || self.delete(&path),
            quarantined,
        )
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
        let recovery = StorageLocation::document_tree(
            uri.as_str(),
            format!("{}.recovery", relative.as_str()),
        )?;
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
            AndroidStorageBackend::document_key(&super::StorageLocation::Native(
                "native.sav".into()
            ))
            .is_err()
        );
    }

    /// Pin shared with the Kotlin side: both crossings assume 64KB.
    /// Change both together; this test fails loudly on drift.
    #[test]
    fn saf_chunk_bytes_matches_kotlin_contract() {
        assert_eq!(super::SAF_CHUNK_BYTES, 64 * 1024);
        assert_eq!(super::SAF_SUMMARY_BYTES, 64 * 1024);
        assert_eq!(
            super::SAF_SUMMARY_BYTES,
            nerust_persistence::slots::MAX_METADATA_BYTES
        );
    }

    #[test]
    fn sized_entry_batch_normalizes_names_and_sizes() {
        let entries: Vec<super::SizedEntry> = serde_json::from_str(
            r#"[{"name":"0.state","size":42},{"name":"1.state"},{"name":"2.state","size":-1},{"name":"3.state","size":10,"summary":"aGk="}]"#,
        )
        .unwrap();
        assert_eq!(entries.len(), 4);
        assert_eq!(entries[0].known_size(), Some(42));
        // Missing sizes are unknown, never a decode failure.
        assert_eq!(entries[1].known_size(), None);
        // Negative Kotlin lengths normalize to unknown.
        assert_eq!(entries[2].known_size(), None);
        // Batched sidecar bytes ride the listing.
        assert_eq!(entries[3].summary.as_deref(), Some("aGk="));
        assert!(entries[0].summary.is_none());
    }

    #[test]
    fn scan_skips_only_known_oversized_files() {
        use super::AndroidStorageBackend;
        assert!(AndroidStorageBackend::scan_candidate(None));
        assert!(AndroidStorageBackend::scan_candidate(Some(42)));
        assert!(AndroidStorageBackend::scan_candidate(Some(
            super::MAX_SLOT_ARCHIVE_BYTES as i64
        )));
        assert!(!AndroidStorageBackend::scan_candidate(Some(
            super::MAX_SLOT_ARCHIVE_BYTES as i64 + 1
        )));
    }
}

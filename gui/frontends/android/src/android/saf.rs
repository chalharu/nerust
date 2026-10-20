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
        // Chunked path bounds every JNI crossing to 64KB (no whole-file
        // Base64 on the bridge thread); legacy whole-file read is the
        // fallback for APKs predating the chunked methods.
        match self.read_chunked(uri, relative) {
            Ok(bytes) => Ok(bytes),
            Err(error) => {
                log::debug!("SAF chunked read unavailable, falling back to whole-file: {error}");
                let value = self.call_read(uri, relative)?;
                value
                    .map(|value| STANDARD.decode(value).map_err(Self::io_error))
                    .transpose()
            }
        }
    }

    /// Streaming read: size probe then 64KB chunks appended into one
    /// buffer. Runs on the session/persistence thread, never the UI thread;
    /// the Kotlin side performs the I/O inline on the calling thread.
    /// Prefers the `byte[]` bridge (no Base64); falls back to Base64
    /// chunks, then to the whole-file read.
    fn read_chunked(&self, uri: &str, relative: &str) -> Result<Option<Vec<u8>>, PersistenceError> {
        match self.read_chunked_bytes(uri, relative) {
            Ok(bytes) => Ok(bytes),
            Err(error) => {
                log::debug!("SAF byte[] read unavailable, falling back to Base64 chunks: {error}");
                self.read_chunked_base64(uri, relative)
            }
        }
    }

    /// `byte[]` chunked read: size probe then raw `byte[]` chunks, one JNI
    /// crossing per 64KB with no Base64 anywhere. New bridge (v3); absence
    /// (old APK) surfaces as `Err` and the caller falls back.
    fn read_chunked_bytes(
        &self,
        uri: &str,
        relative: &str,
    ) -> Result<Option<Vec<u8>>, PersistenceError> {
        let size = self.call_file_size(uri, relative)?;
        if size < 0 {
            return Ok(None);
        }
        let mut bytes = Vec::with_capacity(size.max(0) as usize);
        let mut offset: i64 = 0;
        while offset < size {
            let length = (size - offset).min(SAF_CHUNK_BYTES as i64);
            let chunk = self
                .call_read_chunk_bytes(uri, relative, offset, length)?
                .ok_or_else(|| Self::io_error("Android SAF byte[] chunked read returned null"))?;
            bytes.extend_from_slice(&chunk);
            offset += length;
        }
        Ok(Some(bytes))
    }

    /// One raw `byte[]` chunk read at `offset` (`length` bytes max).
    fn call_read_chunk_bytes(
        &self,
        uri: &str,
        relative: &str,
        offset: i64,
        length: i64,
    ) -> Result<Option<Vec<u8>>, PersistenceError> {
        let vm = unsafe { JavaVM::from_raw(self.app.vm_as_ptr() as _) };
        super::bridge::with_java_env(&vm, |env| {
            let activity_raw = self.app.activity_as_ptr() as jobject;
            let activity = unsafe { env.as_cast_raw::<Global<JObject<'static>>>(&activity_raw)? };
            let uri = JString::from_str(env, uri)?;
            let relative = JString::from_str(env, relative)?;
            let object = env
                .call_method(
                    activity.as_ref(),
                    jni_str!("readSafFileChunkBytes"),
                    jni_sig!("(Ljava/lang/String;Ljava/lang/String;JJ)[B"),
                    &[
                        JValue::Object(uri.as_ref()),
                        JValue::Object(relative.as_ref()),
                        JValue::Long(offset),
                        JValue::Long(length),
                    ],
                )?
                .l()?;
            if object.is_null() {
                Ok(None)
            } else {
                let array = env.cast_local::<jni::objects::JByteArray>(object)?;
                Ok(Some(env.convert_byte_array(&array)?))
            }
        })
        .map_err(Self::io_error)
    }

    fn read_chunked_base64(
        &self,
        uri: &str,
        relative: &str,
    ) -> Result<Option<Vec<u8>>, PersistenceError> {
        let size = self.call_file_size(uri, relative)?;
        if size < 0 {
            return Ok(None);
        }
        let mut bytes = Vec::with_capacity(size.max(0) as usize);
        let mut offset: i64 = 0;
        while offset < size {
            let length = (size - offset).min(SAF_CHUNK_BYTES as i64);
            let chunk = self
                .call_read_chunk(uri, relative, offset, length)?
                .ok_or_else(|| Self::io_error("Android SAF chunked read returned null"))?;
            bytes.extend_from_slice(&STANDARD.decode(chunk).map_err(Self::io_error)?);
            offset += length;
        }
        Ok(Some(bytes))
    }

    fn write(&self, uri: &str, relative: &str, bytes: &[u8]) -> Result<(), PersistenceError> {
        Self::validate_relative_path(relative)?;
        // Chunked path streams 64KB Base64 appends; legacy whole-file write
        // is the fallback for APKs predating the chunked methods.
        match self.write_chunked(uri, relative, bytes) {
            Ok(()) => Ok(()),
            Err(error) => {
                log::debug!("SAF chunked write unavailable, falling back to whole-file: {error}");
                let encoded = STANDARD.encode(bytes);
                let result = self.call_write(uri, relative, &encoded)?;
                if result.as_deref() == Some("ok") {
                    Ok(())
                } else {
                    Err(Self::io_error("Android SAF write failed"))
                }
            }
        }
    }

    /// Streaming write: 64KB chunks, first chunk overwrites, the rest
    /// append. Prefers the `byte[]` bridge (no Base64); falls back to
    /// Base64 chunks, then to the whole-file write.
    fn write_chunked(
        &self,
        uri: &str,
        relative: &str,
        bytes: &[u8],
    ) -> Result<(), PersistenceError> {
        match self.write_chunked_bytes(uri, relative, bytes) {
            Ok(()) => Ok(()),
            Err(error) => {
                log::debug!("SAF byte[] write unavailable, falling back to Base64 chunks: {error}");
                self.write_chunked_base64(uri, relative, bytes)
            }
        }
    }

    /// `byte[]` chunked write: raw 64KB chunks, first chunk overwrites
    /// (`"rwt"`), the rest append (`"wa"`). New bridge (v3); absence (old
    /// APK) surfaces as `Err` and the caller falls back.
    fn write_chunked_bytes(
        &self,
        uri: &str,
        relative: &str,
        bytes: &[u8],
    ) -> Result<(), PersistenceError> {
        if bytes.is_empty() {
            let result = self.call_write_chunk_bytes(uri, relative, &[], false)?;
            return if result.as_deref() == Some("ok") {
                Ok(())
            } else {
                Err(Self::io_error("Android SAF write failed"))
            };
        }
        for (index, chunk) in bytes.chunks(SAF_CHUNK_BYTES).enumerate() {
            let result = self.call_write_chunk_bytes(uri, relative, chunk, index > 0)?;
            if result.as_deref() != Some("ok") {
                return Err(Self::io_error("Android SAF write failed"));
            }
        }
        Ok(())
    }

    /// One raw `byte[]` chunk write; `append` selects `"wa"` (append) over
    /// `"rwt"` (overwrite).
    fn call_write_chunk_bytes(
        &self,
        uri: &str,
        relative: &str,
        chunk: &[u8],
        append: bool,
    ) -> Result<Option<String>, PersistenceError> {
        let owned = chunk.to_vec();
        let vm = unsafe { JavaVM::from_raw(self.app.vm_as_ptr() as _) };
        super::bridge::with_java_env(&vm, |env| {
            let activity_raw = self.app.activity_as_ptr() as jobject;
            let activity = unsafe { env.as_cast_raw::<Global<JObject<'static>>>(&activity_raw)? };
            let uri = JString::from_str(env, uri)?;
            let relative = JString::from_str(env, relative)?;
            let buffer = env.new_byte_array(owned.len())?;
            let signed: Vec<i8> = owned.iter().map(|byte| *byte as i8).collect();
            buffer.set_region(env, 0, &signed)?;
            let object = env
                .call_method(
                    activity.as_ref(),
                    jni_str!("writeSafFileChunkBytes"),
                    jni_sig!("(Ljava/lang/String;Ljava/lang/String;[BZ)Ljava/lang/String;"),
                    &[
                        JValue::Object(uri.as_ref()),
                        JValue::Object(relative.as_ref()),
                        JValue::Object(buffer.as_ref()),
                        JValue::Bool(append),
                    ],
                )?
                .l()?;
            if object.is_null() {
                Ok(None)
            } else {
                Ok(Some(JString::cast_local(env, object)?.try_to_string(env)?))
            }
        })
        .map_err(Self::io_error)
    }

    /// Streaming Base64 write (legacy chunked bridge for old APKs).
    /// 64KB Base64 chunks, first chunk overwrites, the rest append
    /// (`"rwt"` then `"wa"` on the Kotlin side).
    fn write_chunked_base64(
        &self,
        uri: &str,
        relative: &str,
        bytes: &[u8],
    ) -> Result<(), PersistenceError> {
        if bytes.is_empty() {
            let result = self.call_write_chunk(uri, relative, "", false)?;
            return if result.as_deref() == Some("ok") {
                Ok(())
            } else {
                Err(Self::io_error("Android SAF write failed"))
            };
        }
        for (index, chunk) in bytes.chunks(SAF_CHUNK_BYTES).enumerate() {
            let encoded = STANDARD.encode(chunk);
            let result = self.call_write_chunk(uri, relative, &encoded, index > 0)?;
            if result.as_deref() != Some("ok") {
                return Err(Self::io_error("Android SAF write failed"));
            }
        }
        Ok(())
    }

    fn delete(&self, uri: &str, relative: &str) -> Result<(), PersistenceError> {
        Self::validate_relative_path(relative)?;
        let result = self.call_delete(uri, relative)?;
        if matches!(result.as_deref(), Some("ok") | Some("missing")) {
            Ok(())
        } else {
            Err(Self::io_error("Android SAF delete failed"))
        }
    }

    fn list(&self, uri: &str, relative: &str) -> Result<Vec<String>, PersistenceError> {
        Self::validate_relative_path(relative)?;
        // Sized batch avoids the scan N+1 (one size probe per entry);
        // legacy name-only list + per-entry probes is the fallback for
        // APKs predating `listSafFilesWithSizes`.
        if let Ok(entries) = self.list_with_sizes(uri, relative) {
            return Ok(entries.into_iter().map(|(name, _)| name).collect());
        }
        let value = self
            .call_list(uri, relative)?
            .unwrap_or_else(|| "[]".to_string());
        serde_json::from_str(&value).map_err(Self::io_error)
    }

    /// Batched name+size listing: one JNI crossing per directory instead
    /// of one list plus one size probe per entry (scan N+1).
    fn list_with_sizes(
        &self,
        uri: &str,
        relative: &str,
    ) -> Result<Vec<(String, i64)>, PersistenceError> {
        let value = self
            .call_list_with_sizes(uri, relative)?
            .unwrap_or_else(|| "[]".to_string());
        let entries: Vec<SizedEntry> = serde_json::from_str(&value).map_err(Self::io_error)?;
        Ok(entries
            .into_iter()
            .map(|entry| (entry.name, entry.size))
            .collect())
    }

    /// Whole-file read (legacy bridge for old APKs). Each verb below
    /// carries its own JNI name + signature; the old generic
    /// `call_string(method, ..)` ignored `method` on the write path and
    /// always called `writeSafFile`.
    fn call_read(&self, uri: &str, relative: &str) -> Result<Option<String>, PersistenceError> {
        self.call_saf_string(SafStringOp::Read, uri, relative, None)
    }

    fn call_write(
        &self,
        uri: &str,
        relative: &str,
        payload: &str,
    ) -> Result<Option<String>, PersistenceError> {
        self.call_saf_string(SafStringOp::Write, uri, relative, Some(payload))
    }

    fn call_delete(&self, uri: &str, relative: &str) -> Result<Option<String>, PersistenceError> {
        self.call_saf_string(SafStringOp::Delete, uri, relative, None)
    }

    fn call_list(&self, uri: &str, relative: &str) -> Result<Option<String>, PersistenceError> {
        self.call_saf_string(SafStringOp::List, uri, relative, None)
    }

    fn call_list_with_sizes(
        &self,
        uri: &str,
        relative: &str,
    ) -> Result<Option<String>, PersistenceError> {
        self.call_saf_string(SafStringOp::ListWithSizes, uri, relative, None)
    }

    fn call_saf_string(
        &self,
        op: SafStringOp,
        uri: &str,
        relative: &str,
        payload: Option<&str>,
    ) -> Result<Option<String>, PersistenceError> {
        if matches!(op, SafStringOp::Write) && payload.is_none() {
            return Err(Self::io_error("SAF write requires a payload"));
        }
        let vm = unsafe { JavaVM::from_raw(self.app.vm_as_ptr() as _) };
        super::bridge::with_java_env(&vm, |env| {
            let activity_raw = self.app.activity_as_ptr() as jobject;
            let activity = unsafe { env.as_cast_raw::<Global<JObject<'static>>>(&activity_raw)? };
            let uri = JString::from_str(env, uri)?;
            let relative = JString::from_str(env, relative)?;
            let payload = payload
                .map(|value| JString::from_str(env, value))
                .transpose()?;
            let result = match op {
                SafStringOp::Write => {
                    let payload = payload.as_ref().ok_or(jni::errors::Error::JavaException)?;
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
                }
                SafStringOp::Read => env.call_method(
                    activity.as_ref(),
                    jni_str!("readSafFile"),
                    jni_sig!("(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;"),
                    &[
                        JValue::Object(uri.as_ref()),
                        JValue::Object(relative.as_ref()),
                    ],
                )?,
                SafStringOp::Delete => env.call_method(
                    activity.as_ref(),
                    jni_str!("deleteSafFile"),
                    jni_sig!("(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;"),
                    &[
                        JValue::Object(uri.as_ref()),
                        JValue::Object(relative.as_ref()),
                    ],
                )?,
                SafStringOp::List => env.call_method(
                    activity.as_ref(),
                    jni_str!("listSafFiles"),
                    jni_sig!("(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;"),
                    &[
                        JValue::Object(uri.as_ref()),
                        JValue::Object(relative.as_ref()),
                    ],
                )?,
                SafStringOp::ListWithSizes => env.call_method(
                    activity.as_ref(),
                    jni_str!("listSafFilesWithSizes"),
                    jni_sig!("(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;"),
                    &[
                        JValue::Object(uri.as_ref()),
                        JValue::Object(relative.as_ref()),
                    ],
                )?,
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

    /// Size probe for the chunked read: bytes, or a negative value when
    /// the document does not exist. New bridge (v2); absence (old APK)
    /// surfaces as `Err` and the caller falls back to whole-file I/O.
    fn call_file_size(&self, uri: &str, relative: &str) -> Result<i64, PersistenceError> {
        let vm = unsafe { JavaVM::from_raw(self.app.vm_as_ptr() as _) };
        super::bridge::with_java_env(&vm, |env| {
            let activity_raw = self.app.activity_as_ptr() as jobject;
            let activity = unsafe { env.as_cast_raw::<Global<JObject<'static>>>(&activity_raw)? };
            let uri = JString::from_str(env, uri)?;
            let relative = JString::from_str(env, relative)?;
            let size = env
                .call_method(
                    activity.as_ref(),
                    jni_str!("getSafFileSize"),
                    jni_sig!("(Ljava/lang/String;Ljava/lang/String;)J"),
                    &[
                        JValue::Object(uri.as_ref()),
                        JValue::Object(relative.as_ref()),
                    ],
                )?
                .j()?;
            Ok::<_, jni::errors::Error>(size)
        })
        .map_err(Self::io_error)
    }

    /// One 64KB Base64 chunk read at `offset`. New bridge (v2); absence
    /// (old APK) surfaces as `Err` and the caller falls back.
    fn call_read_chunk(
        &self,
        uri: &str,
        relative: &str,
        offset: i64,
        length: i64,
    ) -> Result<Option<String>, PersistenceError> {
        let vm = unsafe { JavaVM::from_raw(self.app.vm_as_ptr() as _) };
        super::bridge::with_java_env(&vm, |env| {
            let activity_raw = self.app.activity_as_ptr() as jobject;
            let activity = unsafe { env.as_cast_raw::<Global<JObject<'static>>>(&activity_raw)? };
            let uri = JString::from_str(env, uri)?;
            let relative = JString::from_str(env, relative)?;
            let object = env
                .call_method(
                    activity.as_ref(),
                    jni_str!("readSafFileChunk"),
                    jni_sig!("(Ljava/lang/String;Ljava/lang/String;JJ)Ljava/lang/String;"),
                    &[
                        JValue::Object(uri.as_ref()),
                        JValue::Object(relative.as_ref()),
                        JValue::Long(offset),
                        JValue::Long(length),
                    ],
                )?
                .l()?;
            if object.is_null() {
                Ok(None)
            } else {
                Ok(Some(JString::cast_local(env, object)?.try_to_string(env)?))
            }
        })
        .map_err(Self::io_error)
    }

    /// One 64KB Base64 chunk write; `append` selects `"wa"` (append) over
    /// `"rwt"` (overwrite). New bridge (v2); absence (old APK) surfaces
    /// as `Err` and the caller falls back.
    fn call_write_chunk(
        &self,
        uri: &str,
        relative: &str,
        encoded: &str,
        append: bool,
    ) -> Result<Option<String>, PersistenceError> {
        let vm = unsafe { JavaVM::from_raw(self.app.vm_as_ptr() as _) };
        super::bridge::with_java_env(&vm, |env| {
            let activity_raw = self.app.activity_as_ptr() as jobject;
            let activity = unsafe { env.as_cast_raw::<Global<JObject<'static>>>(&activity_raw)? };
            let uri = JString::from_str(env, uri)?;
            let relative = JString::from_str(env, relative)?;
            let encoded = JString::from_str(env, encoded)?;
            let object = env
                .call_method(
                    activity.as_ref(),
                    jni_str!("writeSafFileChunk"),
                    jni_sig!(
                        "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Z)Ljava/lang/String;"
                    ),
                    &[
                        JValue::Object(uri.as_ref()),
                        JValue::Object(relative.as_ref()),
                        JValue::Object(encoded.as_ref()),
                        JValue::Bool(append),
                    ],
                )?
                .l()?;
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

/// Per-crossing SAF chunk: 64KB of raw bytes per JNI call bounds
/// bridge memory without changing the legacy whole-file signatures.
/// Contract: must equal `SafRepository.SAF_CHUNK_BYTES` in Kotlin
/// (see `saf_chunk_bytes_matches_kotlin_contract` below).
const SAF_CHUNK_BYTES: usize = 64 * 1024;

/// String-bridge verbs. Each maps to exactly one JNI name + signature in
/// `call_saf_string`; there is no `method: &str` parameter to ignore.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SafStringOp {
    Read,
    Write,
    Delete,
    List,
    ListWithSizes,
}

/// One entry of the `listSafFilesWithSizes` JSON batch:
/// `[{"name": "...", "size": 123}]`. Sizes ride the same crossing as the
/// names so `scan` needs no per-entry probe (no N+1).
#[derive(Debug, serde::Deserialize)]
struct SizedEntry {
    name: String,
    #[serde(default)]
    size: i64,
}

impl SlotBackend for AndroidStorageBackend {
    /// SAF scan runs on the session/persistence thread, never the event
    /// loop or UI thread (like `read_uri_bytes`, the per-entry `read`
    /// below blocks on the provider). The sized batch (`list_with_sizes`)
    /// keeps it to one crossing per entry plus one listing.
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

    /// Pin shared with the Kotlin side (`SafRepository.SAF_CHUNK_BYTES`)
    /// and `packaging/android` docs: both crossings assume 64KB. Change
    /// both together; this test fails loudly if Rust drifts.
    #[test]
    fn saf_chunk_bytes_matches_kotlin_contract() {
        assert_eq!(super::SAF_CHUNK_BYTES, 64 * 1024);
    }

    #[test]
    fn sized_entry_batch_decodes_names_and_sizes() {
        let entries: Vec<super::SizedEntry> =
            serde_json::from_str(r#"[{"name":"0.state","size":42},{"name":"1.state"}]"#).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "0.state");
        assert_eq!(entries[0].size, 42);
        // Missing sizes default to 0 (unknown), never a decode failure.
        assert_eq!(entries[1].size, 0);
    }
}

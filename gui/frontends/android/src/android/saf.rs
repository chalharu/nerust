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
        // Streamed session first: one openInputStream, sequential server-side
        // reads. Chunked and whole-file paths stay for old APKs.
        match self.read_streamed(uri, relative) {
            Ok(bytes) => return Ok(bytes),
            Err(error) => {
                log::debug!("SAF streamed read unavailable, falling back to chunked: {error}");
            }
        }
        // Chunked path bounds every JNI crossing to 64KB.
        // Legacy whole-file read is the fallback for old APKs.
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

    /// Streaming read: size probe then 64KB chunks into one buffer.
    /// Prefers `byte[]`; falls back to Base64 chunks, then whole-file.
    fn read_chunked(&self, uri: &str, relative: &str) -> Result<Option<Vec<u8>>, PersistenceError> {
        match self.read_chunked_bytes(uri, relative) {
            Ok(bytes) => Ok(bytes),
            Err(error) => {
                log::debug!("SAF byte[] read unavailable, falling back to Base64 chunks: {error}");
                self.read_chunked_base64(uri, relative)
            }
        }
    }

    /// Sized read: skips the size probe when the batch listing carries the length.
    /// Prefers `byte[]`; falls back to Base64 chunks.
    fn read_sized(
        &self,
        uri: &str,
        relative: &str,
        size: i64,
    ) -> Result<Option<Vec<u8>>, PersistenceError> {
        Self::validate_relative_path(relative)?;
        if size < 0 {
            return Ok(None);
        }
        match self.read_chunked_bytes_sized(uri, relative, size) {
            Ok(bytes) => Ok(bytes),
            Err(error) => {
                log::debug!("SAF byte[] read unavailable, falling back to Base64 chunks: {error}");
                self.read_chunked_base64_sized(uri, relative, size)
            }
        }
    }

    /// `byte[]` chunked read: size probe then raw chunks, one JNI crossing per 64KB.
    /// Absence (old APK) surfaces as `Err`; the caller falls back.
    fn read_chunked_bytes(
        &self,
        uri: &str,
        relative: &str,
    ) -> Result<Option<Vec<u8>>, PersistenceError> {
        let size = self.call_file_size(uri, relative)?;
        self.read_chunked_bytes_sized(uri, relative, size)
    }

    /// `byte[]` chunked read with a known size: no probe crossing.
    fn read_chunked_bytes_sized(
        &self,
        uri: &str,
        relative: &str,
        size: i64,
    ) -> Result<Option<Vec<u8>>, PersistenceError> {
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
        self.read_chunked_base64_sized(uri, relative, size)
    }

    /// Base64 chunked read with a known size: no probe crossing.
    fn read_chunked_base64_sized(
        &self,
        uri: &str,
        relative: &str,
        size: i64,
    ) -> Result<Option<Vec<u8>>, PersistenceError> {
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

    /// Streaming session read: one `openInputStream`, sequential server-side
    /// chunks in a single pass. Absence (old APK) surfaces as `Err`.
    fn read_streamed(
        &self,
        uri: &str,
        relative: &str,
    ) -> Result<Option<Vec<u8>>, PersistenceError> {
        let session = self.call_open_read_session(uri, relative)?;
        if session < 0 {
            return Ok(None);
        }
        let mut bytes = Vec::new();
        let result = (|| {
            loop {
                match self.call_read_session_chunk(session)? {
                    Some(chunk) if !chunk.is_empty() => bytes.extend_from_slice(&chunk),
                    _ => break,
                }
            }
            Ok::<(), PersistenceError>(())
        })();
        let close_result = self.call_close_read_session(session);
        result?;
        close_result?;
        Ok(Some(bytes))
    }

    /// Open a server-side read session; negative means the document is missing.
    fn call_open_read_session(&self, uri: &str, relative: &str) -> Result<i64, PersistenceError> {
        let vm = unsafe { JavaVM::from_raw(self.app.vm_as_ptr() as _) };
        super::bridge::with_java_env(&vm, |env| {
            let activity_raw = self.app.activity_as_ptr() as jobject;
            let activity = unsafe { env.as_cast_raw::<Global<JObject<'static>>>(&activity_raw)? };
            let uri = JString::from_str(env, uri)?;
            let relative = JString::from_str(env, relative)?;
            let session = env
                .call_method(
                    activity.as_ref(),
                    jni_str!("openSafReadSession"),
                    jni_sig!("(Ljava/lang/String;Ljava/lang/String;)J"),
                    &[
                        JValue::Object(uri.as_ref()),
                        JValue::Object(relative.as_ref()),
                    ],
                )?
                .j()?;
            Ok::<_, jni::errors::Error>(session)
        })
        .map_err(Self::io_error)
    }

    /// One sequential chunk from an open session; empty marks end of stream.
    fn call_read_session_chunk(&self, session: i64) -> Result<Option<Vec<u8>>, PersistenceError> {
        let vm = unsafe { JavaVM::from_raw(self.app.vm_as_ptr() as _) };
        super::bridge::with_java_env(&vm, |env| {
            let activity_raw = self.app.activity_as_ptr() as jobject;
            let activity = unsafe { env.as_cast_raw::<Global<JObject<'static>>>(&activity_raw)? };
            let object = env
                .call_method(
                    activity.as_ref(),
                    jni_str!("readSafReadSession"),
                    jni_sig!("(JI)[B"),
                    &[JValue::Long(session), JValue::Int(SAF_CHUNK_BYTES as i32)],
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

    /// Close a server-side read session; a missing session is a no-op.
    fn call_close_read_session(&self, session: i64) -> Result<(), PersistenceError> {
        let vm = unsafe { JavaVM::from_raw(self.app.vm_as_ptr() as _) };
        super::bridge::with_java_env(&vm, |env| {
            let activity_raw = self.app.activity_as_ptr() as jobject;
            let activity = unsafe { env.as_cast_raw::<Global<JObject<'static>>>(&activity_raw)? };
            env.call_method(
                activity.as_ref(),
                jni_str!("closeSafReadSession"),
                jni_sig!("(J)V"),
                &[JValue::Long(session)],
            )?;
            Ok::<_, jni::errors::Error>(())
        })
        .map_err(Self::io_error)
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

    /// Streaming write: first chunk overwrites, the rest append.
    /// Prefers `byte[]`; falls back to Base64 chunks, then whole-file.
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

    /// `byte[]` chunked write: first chunk overwrites, the rest append.
    /// Absence (old APK) surfaces as `Err`; the caller falls back.
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
        let vm = unsafe { JavaVM::from_raw(self.app.vm_as_ptr() as _) };
        super::bridge::with_java_env(&vm, |env| {
            let activity_raw = self.app.activity_as_ptr() as jobject;
            let activity = unsafe { env.as_cast_raw::<Global<JObject<'static>>>(&activity_raw)? };
            let uri = JString::from_str(env, uri)?;
            let relative = JString::from_str(env, relative)?;
            let buffer = env.byte_array_from_slice(chunk)?;
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
    /// First chunk overwrites, the rest append.
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

    /// Batched listing: one JNI crossing per directory; negatives mean unknown.
    fn list_with_sizes(
        &self,
        uri: &str,
        relative: &str,
    ) -> Result<Vec<(String, Option<i64>)>, PersistenceError> {
        let value = self
            .call_list_with_sizes(uri, relative)?
            .unwrap_or_else(|| "[]".to_string());
        let entries: Vec<SizedEntry> = serde_json::from_str(&value).map_err(Self::io_error)?;
        Ok(entries
            .into_iter()
            .map(|entry| {
                let size = entry.known_size();
                (entry.name, size)
            })
            .collect())
    }

    /// One batched listing shared by `scan` and `allocate_next_id`.
    /// Sized batch first; names-only list is the old-APK fallback.
    fn sized_listing(
        &self,
        uri: &str,
        relative: &str,
    ) -> Result<Vec<(String, Option<i64>)>, PersistenceError> {
        if let Ok(entries) = self.list_with_sizes(uri, relative) {
            return Ok(entries);
        }
        let value = self
            .call_list(uri, relative)?
            .unwrap_or_else(|| "[]".to_string());
        serde_json::from_str::<Vec<String>>(&value)
            .map_err(Self::io_error)
            .map(|names| names.into_iter().map(|name| (name, None)).collect())
    }

    /// Whole-file read (legacy bridge for old APKs).
    /// Each verb below carries its own JNI name + signature.
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

    /// Size probe for the chunked read: bytes, negative when missing.
    /// Absence (old APK) surfaces as `Err`; the caller falls back.
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

    /// One 64KB Base64 chunk write; `append` selects append over overwrite.
    /// Absence (old APK) surfaces as `Err`; the caller falls back.
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

/// Per-crossing SAF chunk: 64KB of raw bytes per JNI call.
/// Must equal `SafRepository.SAF_CHUNK_BYTES` in Kotlin.
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
            return FsSlotBackend.scan(dir, identity);
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
            let bytes = match size {
                Some(size) => self.read_sized(&uri, &child, size)?,
                None => self.read(&uri, &child)?,
            };
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
            return FsSlotBackend.allocate_next_id(dir);
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

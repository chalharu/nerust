use std::{
    io::Error as IoError,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
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
use crate::saf_decision::{SafCapability, SafError};

#[derive(Clone)]
pub(super) struct AndroidStorageBackend {
    app: AndroidApp,
    /// Memoized probe winner: every transfer takes this path.
    /// A skew on the memoized path re-probes once; I/O faults hard-error.
    capability: Arc<Mutex<Option<SafCapability>>>,
}

impl AndroidStorageBackend {
    pub(super) fn new(app: AndroidApp) -> Self {
        Self {
            app,
            capability: Arc::new(Mutex::new(None)),
        }
    }

    /// Classify a JNI failure: only a missing method is a version skew.
    /// Every other fault hard-errors on the chosen path.
    fn classify(error: jni::errors::Error) -> SafError {
        match error {
            jni::errors::Error::MethodNotFound { .. }
            | jni::errors::Error::NoSuchMethod(_)
            | jni::errors::Error::FieldNotFound { .. }
            | jni::errors::Error::EnvMethodNotFound(_) => SafError::version_skew(),
            other => SafError::io(other.to_string()),
        }
    }

    fn persist(error: SafError) -> PersistenceError {
        PersistenceError::Io(match error {
            SafError::Missing => IoError::other("Android SAF document is missing"),
            SafError::VersionSkew => {
                IoError::other("Android SAF bridge is older than the chosen path")
            }
            SafError::Io(message) => IoError::other(message),
        })
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

    fn memo(&self) -> Option<SafCapability> {
        self.capability.lock().map(|guard| *guard).unwrap_or(None)
    }

    fn memoize(&self, capability: SafCapability) {
        if let Ok(mut guard) = self.capability.lock() {
            *guard = Some(capability);
        }
    }

    fn clear_memo(&self) {
        if let Ok(mut guard) = self.capability.lock() {
            *guard = None;
        }
    }

    fn read(&self, uri: &str, relative: &str) -> Result<Option<Vec<u8>>, PersistenceError> {
        Self::validate_relative_path(relative)?;
        // Absent documents read as `None`, never as I/O faults.
        match self.run_read(uri, relative) {
            Err(SafError::Missing) => Ok(None),
            outcome => outcome.map_err(Self::persist),
        }
    }

    /// One memoized path; a skew re-probes once, faults hard-error.
    fn run_read(&self, uri: &str, relative: &str) -> Result<Option<Vec<u8>>, SafError> {
        if let Some(capability) = self.memo() {
            match self.read_with(capability, uri, relative) {
                Err(error) if error.fallback_eligible() => self.clear_memo(),
                outcome => return outcome,
            }
        }
        self.probe_read(uri, relative)
    }

    /// Single probe: first non-skew path wins and memoizes.
    fn probe_read(&self, uri: &str, relative: &str) -> Result<Option<Vec<u8>>, SafError> {
        for capability in [
            SafCapability::Streamed,
            SafCapability::ChunkedBytes,
            SafCapability::ChunkedBase64,
            SafCapability::Legacy,
        ] {
            match self.read_with(capability, uri, relative) {
                Err(error) if error.fallback_eligible() => continue,
                outcome => {
                    self.memoize(capability);
                    return outcome;
                }
            }
        }
        Err(SafError::version_skew())
    }

    fn read_with(
        &self,
        capability: SafCapability,
        uri: &str,
        relative: &str,
    ) -> Result<Option<Vec<u8>>, SafError> {
        match capability {
            SafCapability::Streamed => self.read_streamed(uri, relative),
            SafCapability::ChunkedBytes => self.read_chunked_bytes(uri, relative),
            SafCapability::ChunkedBase64 => self.read_chunked_base64(uri, relative),
            SafCapability::Legacy => match self.call_read(uri, relative)? {
                None => Err(SafError::missing()),
                Some(value) => Ok(Some(STANDARD.decode(value).map_err(SafError::io)?)),
            },
        }
    }

    /// Sized read: skips the size probe when the batch listing carries the length.
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
        let capability = self
            .memo()
            .unwrap_or(SafCapability::ChunkedBytes)
            .write_capability();
        match self.read_sized_with(capability, uri, relative, size) {
            Err(SafError::Missing) => Ok(None),
            outcome => outcome.map_err(Self::persist),
        }
    }

    fn read_sized_with(
        &self,
        capability: SafCapability,
        uri: &str,
        relative: &str,
        size: i64,
    ) -> Result<Option<Vec<u8>>, SafError> {
        match capability {
            SafCapability::ChunkedBytes | SafCapability::Streamed => {
                self.read_chunked_bytes_sized(uri, relative, size)
            }
            SafCapability::ChunkedBase64 => self.read_chunked_base64_sized(uri, relative, size),
            SafCapability::Legacy => match self.call_read(uri, relative)? {
                None => Err(SafError::missing()),
                Some(value) => Ok(Some(STANDARD.decode(value).map_err(SafError::io)?)),
            },
        }
    }

    /// `byte[]` chunked read: size probe then raw chunks, one JNI crossing per 64KB.
    /// One attach per transfer; the URI strings intern once for every chunk.
    fn read_chunked_bytes(&self, uri: &str, relative: &str) -> Result<Option<Vec<u8>>, SafError> {
        let size = self.call_file_size(uri, relative)?;
        self.read_chunked_bytes_sized(uri, relative, size)
    }

    fn read_chunked_bytes_sized(
        &self,
        uri: &str,
        relative: &str,
        size: i64,
    ) -> Result<Option<Vec<u8>>, SafError> {
        if size < 0 {
            return Err(SafError::missing());
        }
        let vm = unsafe { JavaVM::from_raw(self.app.vm_as_ptr() as _) };
        bridge::with_java_env(&vm, |env| {
            let activity_raw = self.app.activity_as_ptr() as jobject;
            let activity = unsafe { env.as_cast_raw::<Global<JObject<'static>>>(&activity_raw)? };
            let activity = activity.as_ref();
            let uri = JString::from_str(env, uri)?;
            let relative = JString::from_str(env, relative)?;
            let mut bytes = Vec::with_capacity(size.max(0) as usize);
            let mut offset: i64 = 0;
            while offset < size {
                let length = (size - offset).min(SAF_CHUNK_BYTES as i64);
                let chunk = Self::call_read_chunk_bytes_inner(
                    env, activity, &uri, &relative, offset, length,
                )?
                .ok_or(jni::errors::Error::NullPtr(
                    "Android SAF byte[] chunked read returned null",
                ))?;
                bytes.extend_from_slice(&chunk);
                offset += length;
            }
            Ok::<_, jni::errors::Error>(Some(bytes))
        })
        .map_err(Self::classify)
    }

    /// One raw `byte[]` chunk read at `offset` (`length` bytes max).
    fn call_read_chunk_bytes_inner(
        env: &mut jni::Env<'_>,
        activity: &JObject<'_>,
        uri: &JString<'_>,
        relative: &JString<'_>,
        offset: i64,
        length: i64,
    ) -> jni::errors::Result<Option<Vec<u8>>> {
        let object = bridge::call_bridge_method(
            env,
            activity,
            &bridge::methods::READ_SAF_FILE_CHUNK_BYTES,
            &[
                JValue::Object(uri.as_ref()),
                JValue::Object(relative.as_ref()),
                JValue::Long(offset),
                JValue::Long(length),
            ],
        )?
        .l()?;
        if object.is_null() {
            return Ok(None);
        }
        let array = env.cast_local::<jni::objects::JByteArray>(object)?;
        Ok(Some(env.convert_byte_array(&array)?))
    }

    fn read_chunked_base64(&self, uri: &str, relative: &str) -> Result<Option<Vec<u8>>, SafError> {
        let size = self.call_file_size(uri, relative)?;
        self.read_chunked_base64_sized(uri, relative, size)
    }

    /// Base64 chunked read with a known size: one attach, interned strings.
    fn read_chunked_base64_sized(
        &self,
        uri: &str,
        relative: &str,
        size: i64,
    ) -> Result<Option<Vec<u8>>, SafError> {
        if size < 0 {
            return Ok(None);
        }
        let vm = unsafe { JavaVM::from_raw(self.app.vm_as_ptr() as _) };
        bridge::with_java_env(&vm, |env| {
            let activity_raw = self.app.activity_as_ptr() as jobject;
            let activity = unsafe { env.as_cast_raw::<Global<JObject<'static>>>(&activity_raw)? };
            let activity = activity.as_ref();
            let uri = JString::from_str(env, uri)?;
            let relative = JString::from_str(env, relative)?;
            let mut bytes = Vec::with_capacity(size.max(0) as usize);
            let mut offset: i64 = 0;
            while offset < size {
                let length = (size - offset).min(SAF_CHUNK_BYTES as i64);
                let chunk =
                    Self::call_read_chunk_inner(env, activity, &uri, &relative, offset, length)?
                        .ok_or(jni::errors::Error::NullPtr(
                            "Android SAF chunked read returned null",
                        ))?;
                bytes.extend_from_slice(
                    &STANDARD
                        .decode(chunk)
                        .map_err(|error| jni::errors::Error::ParseFailed(error.to_string()))?,
                );
                offset += length;
            }
            Ok::<_, jni::errors::Error>(Some(bytes))
        })
        .map_err(Self::classify)
    }

    /// Streaming read: one `openInputStream`, sequential server-side
    /// chunks in a single pass and attach. Absence surfaces as skew.
    fn read_streamed(&self, uri: &str, relative: &str) -> Result<Option<Vec<u8>>, SafError> {
        let vm = unsafe { JavaVM::from_raw(self.app.vm_as_ptr() as _) };
        bridge::with_java_env(&vm, |env| {
            let activity_raw = self.app.activity_as_ptr() as jobject;
            let activity = unsafe { env.as_cast_raw::<Global<JObject<'static>>>(&activity_raw)? };
            let activity = activity.as_ref();
            let uri = JString::from_str(env, uri)?;
            let relative = JString::from_str(env, relative)?;
            let session = Self::call_open_read_session_inner(env, activity, &uri, &relative)?;
            if session < 0 {
                return Ok(None);
            }
            let mut bytes = Vec::new();
            let result = (|| {
                loop {
                    match Self::call_read_session_chunk_inner(env, activity, session)? {
                        Some(chunk) if !chunk.is_empty() => bytes.extend_from_slice(&chunk),
                        _ => break,
                    }
                }
                Ok::<(), jni::errors::Error>(())
            })();
            let close_result = Self::call_close_read_session_inner(env, activity, session);
            result?;
            close_result?;
            Ok::<_, jni::errors::Error>(Some(bytes))
        })
        .map_err(Self::classify)
    }

    /// Open a server-side read session; negative means the document is missing.
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
    ) -> jni::errors::Result<Option<Vec<u8>>> {
        let object = bridge::call_bridge_method(
            env,
            activity,
            &bridge::methods::READ_SAF_READ_SESSION,
            &[JValue::Long(session), JValue::Int(SAF_CHUNK_BYTES as i32)],
        )?
        .l()?;
        if object.is_null() {
            return Ok(None);
        }
        let array = env.cast_local::<jni::objects::JByteArray>(object)?;
        Ok(Some(env.convert_byte_array(&array)?))
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
        self.run_write(uri, relative, bytes).map_err(Self::persist)
    }

    /// One memoized path; a skew re-probes once, faults hard-error.
    fn run_write(&self, uri: &str, relative: &str, bytes: &[u8]) -> Result<(), SafError> {
        if let Some(capability) = self.memo() {
            match self.write_with(capability.write_capability(), uri, relative, bytes) {
                Err(error) if error.fallback_eligible() => self.clear_memo(),
                outcome => return outcome,
            }
        }
        self.probe_write(uri, relative, bytes)
    }

    /// Single probe: first non-skew path wins and memoizes.
    fn probe_write(&self, uri: &str, relative: &str, bytes: &[u8]) -> Result<(), SafError> {
        for capability in [
            SafCapability::ChunkedBytes,
            SafCapability::ChunkedBase64,
            SafCapability::Legacy,
        ] {
            match self.write_with(capability, uri, relative, bytes) {
                Err(error) if error.fallback_eligible() => continue,
                outcome => {
                    self.memoize(capability);
                    return outcome;
                }
            }
        }
        Err(SafError::version_skew())
    }

    fn write_with(
        &self,
        capability: SafCapability,
        uri: &str,
        relative: &str,
        bytes: &[u8],
    ) -> Result<(), SafError> {
        match capability {
            SafCapability::Streamed | SafCapability::ChunkedBytes => {
                self.write_chunked_bytes(uri, relative, bytes)
            }
            SafCapability::ChunkedBase64 => self.write_chunked_base64(uri, relative, bytes),
            SafCapability::Legacy => {
                let encoded = STANDARD.encode(bytes);
                let result = self.call_write(uri, relative, &encoded)?;
                if result.as_deref() == Some("ok") {
                    Ok(())
                } else {
                    Err(SafError::io("Android SAF write failed"))
                }
            }
        }
    }

    /// `byte[]` chunked write: first chunk overwrites, the rest append.
    /// One attach per transfer; the URI strings intern once for every chunk.
    fn write_chunked_bytes(&self, uri: &str, relative: &str, bytes: &[u8]) -> Result<(), SafError> {
        let vm = unsafe { JavaVM::from_raw(self.app.vm_as_ptr() as _) };
        bridge::with_java_env(&vm, |env| {
            let activity_raw = self.app.activity_as_ptr() as jobject;
            let activity = unsafe { env.as_cast_raw::<Global<JObject<'static>>>(&activity_raw)? };
            let activity = activity.as_ref();
            let uri = JString::from_str(env, uri)?;
            let relative = JString::from_str(env, relative)?;
            let chunks: Vec<&[u8]> = if bytes.is_empty() {
                vec![&[]]
            } else {
                bytes.chunks(SAF_CHUNK_BYTES).collect()
            };
            for (index, chunk) in chunks.iter().enumerate() {
                let result = Self::call_write_chunk_bytes_inner(
                    env,
                    activity,
                    &uri,
                    &relative,
                    chunk,
                    index > 0,
                )?;
                if result.as_deref() != Some("ok") {
                    return Err(jni::errors::Error::NullPtr("Android SAF write failed"));
                }
            }
            Ok::<(), jni::errors::Error>(())
        })
        .map_err(Self::classify)
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
    ) -> jni::errors::Result<Option<String>> {
        let buffer = env.byte_array_from_slice(chunk)?;
        let object = bridge::call_bridge_method(
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
        .l()?;
        if object.is_null() {
            return Ok(None);
        }
        Ok(Some(JString::cast_local(env, object)?.try_to_string(env)?))
    }

    /// Streaming Base64 write (legacy chunked bridge for old APKs).
    /// First chunk overwrites, the rest append; one attach per transfer.
    fn write_chunked_base64(
        &self,
        uri: &str,
        relative: &str,
        bytes: &[u8],
    ) -> Result<(), SafError> {
        let vm = unsafe { JavaVM::from_raw(self.app.vm_as_ptr() as _) };
        bridge::with_java_env(&vm, |env| {
            let activity_raw = self.app.activity_as_ptr() as jobject;
            let activity = unsafe { env.as_cast_raw::<Global<JObject<'static>>>(&activity_raw)? };
            let activity = activity.as_ref();
            let uri = JString::from_str(env, uri)?;
            let relative = JString::from_str(env, relative)?;
            let chunks: Vec<Vec<u8>> = if bytes.is_empty() {
                vec![Vec::new()]
            } else {
                bytes.chunks(SAF_CHUNK_BYTES).map(<[u8]>::to_vec).collect()
            };
            for (index, chunk) in chunks.iter().enumerate() {
                let encoded = STANDARD.encode(chunk);
                let encoded = JString::from_str(env, &encoded)?;
                let result = Self::call_write_chunk_inner(
                    env,
                    activity,
                    &uri,
                    &relative,
                    &encoded,
                    index > 0,
                )?;
                if result.as_deref() != Some("ok") {
                    return Err(jni::errors::Error::NullPtr("Android SAF write failed"));
                }
            }
            Ok::<(), jni::errors::Error>(())
        })
        .map_err(Self::classify)
    }

    fn delete(&self, uri: &str, relative: &str) -> Result<(), PersistenceError> {
        Self::validate_relative_path(relative)?;
        let result = self.call_delete(uri, relative).map_err(Self::persist)?;
        if matches!(result.as_deref(), Some("ok") | Some("missing")) {
            Ok(())
        } else {
            Err(PersistenceError::Io(IoError::other(
                "Android SAF delete failed",
            )))
        }
    }

    /// Batched listing: one JNI crossing per directory; negatives mean unknown.
    /// Skew surfaces typed so the caller falls back exactly once.
    fn list_with_sizes(
        &self,
        uri: &str,
        relative: &str,
    ) -> Result<Vec<(String, Option<i64>)>, SafError> {
        let value = self
            .call_list_with_sizes(uri, relative)?
            .unwrap_or_else(|| "[]".to_string());
        let entries: Vec<SizedEntry> = serde_json::from_str(&value).map_err(SafError::io)?;
        Ok(entries
            .into_iter()
            .map(|entry| {
                let size = entry.known_size();
                (entry.name, size)
            })
            .collect())
    }

    /// One batched listing shared by `scan` and `allocate_next_id`.
    /// Sized batch first; only a version skew falls back.
    fn sized_listing(
        &self,
        uri: &str,
        relative: &str,
    ) -> Result<Vec<(String, Option<i64>)>, PersistenceError> {
        match self.list_with_sizes(uri, relative) {
            Ok(entries) => Ok(entries),
            Err(error) if error.fallback_eligible() => {
                let value = self
                    .call_list(uri, relative)
                    .map_err(Self::persist)?
                    .unwrap_or_else(|| "[]".to_string());
                serde_json::from_str::<Vec<String>>(&value)
                    .map_err(|error| Self::persist(SafError::io(error)))
                    .map(|names| names.into_iter().map(|name| (name, None)).collect())
            }
            Err(error) => Err(Self::persist(error)),
        }
    }

    /// String-bridge verbs. Each maps to exactly one tabled JNI name + signature.
    fn call_read(&self, uri: &str, relative: &str) -> Result<Option<String>, SafError> {
        self.call_saf_string(&bridge::methods::READ_SAF_FILE, uri, relative, None)
    }

    fn call_write(
        &self,
        uri: &str,
        relative: &str,
        payload: &str,
    ) -> Result<Option<String>, SafError> {
        self.call_saf_string(
            &bridge::methods::WRITE_SAF_FILE,
            uri,
            relative,
            Some(payload),
        )
    }

    fn call_delete(&self, uri: &str, relative: &str) -> Result<Option<String>, SafError> {
        self.call_saf_string(&bridge::methods::DELETE_SAF_FILE, uri, relative, None)
    }

    fn call_list(&self, uri: &str, relative: &str) -> Result<Option<String>, SafError> {
        self.call_saf_string(&bridge::methods::LIST_SAF_FILES, uri, relative, None)
    }

    fn call_list_with_sizes(&self, uri: &str, relative: &str) -> Result<Option<String>, SafError> {
        self.call_saf_string(
            &bridge::methods::LIST_SAF_FILES_WITH_SIZES,
            uri,
            relative,
            None,
        )
    }

    fn call_saf_string(
        &self,
        method: &bridge::methods::BridgeMethod,
        uri: &str,
        relative: &str,
        payload: Option<&str>,
    ) -> Result<Option<String>, SafError> {
        // `call_write` is the only payload carrier; its signature
        // takes `&str`, so a missing payload cannot typecheck.
        let vm = unsafe { JavaVM::from_raw(self.app.vm_as_ptr() as _) };
        bridge::with_java_env(&vm, |env| {
            let activity_raw = self.app.activity_as_ptr() as jobject;
            let activity = unsafe { env.as_cast_raw::<Global<JObject<'static>>>(&activity_raw)? };
            let activity = activity.as_ref();
            let uri = JString::from_str(env, uri)?;
            let relative = JString::from_str(env, relative)?;
            let payload = payload
                .map(|value| JString::from_str(env, value))
                .transpose()?;
            let mut args = vec![
                JValue::Object(uri.as_ref()),
                JValue::Object(relative.as_ref()),
            ];
            if let Some(payload) = payload.as_ref() {
                args.push(JValue::Object(payload.as_ref()));
            }
            let object = bridge::call_bridge_method(env, activity, method, &args)?.l()?;
            if object.is_null() {
                Ok(None)
            } else {
                Ok(Some(JString::cast_local(env, object)?.try_to_string(env)?))
            }
        })
        .map_err(Self::classify)
    }

    /// Size probe for the chunked read: bytes, negative when missing.
    /// A missing method surfaces as skew; the probe picks another path.
    fn call_file_size(&self, uri: &str, relative: &str) -> Result<i64, SafError> {
        let vm = unsafe { JavaVM::from_raw(self.app.vm_as_ptr() as _) };
        bridge::with_java_env(&vm, |env| {
            let activity_raw = self.app.activity_as_ptr() as jobject;
            let activity = unsafe { env.as_cast_raw::<Global<JObject<'static>>>(&activity_raw)? };
            let activity = activity.as_ref();
            let uri = JString::from_str(env, uri)?;
            let relative = JString::from_str(env, relative)?;
            Ok::<_, jni::errors::Error>(
                bridge::call_bridge_method(
                    env,
                    activity,
                    &bridge::methods::GET_SAF_FILE_SIZE,
                    &[
                        JValue::Object(uri.as_ref()),
                        JValue::Object(relative.as_ref()),
                    ],
                )?
                .j()?,
            )
        })
        .map_err(Self::classify)
    }

    /// One 64KB Base64 chunk read at `offset`, through the shared attach.
    /// Absence surfaces as skew and the probe picks another path.
    fn call_read_chunk_inner(
        env: &mut jni::Env<'_>,
        activity: &JObject<'_>,
        uri: &JString<'_>,
        relative: &JString<'_>,
        offset: i64,
        length: i64,
    ) -> jni::errors::Result<Option<String>> {
        let object = bridge::call_bridge_method(
            env,
            activity,
            &bridge::methods::READ_SAF_FILE_CHUNK,
            &[
                JValue::Object(uri.as_ref()),
                JValue::Object(relative.as_ref()),
                JValue::Long(offset),
                JValue::Long(length),
            ],
        )?
        .l()?;
        if object.is_null() {
            return Ok(None);
        }
        Ok(Some(JString::cast_local(env, object)?.try_to_string(env)?))
    }

    /// One 64KB Base64 chunk write; `append` selects append over overwrite.
    /// Absence surfaces as skew; the probe picks another path.
    fn call_write_chunk_inner(
        env: &mut jni::Env<'_>,
        activity: &JObject<'_>,
        uri: &JString<'_>,
        relative: &JString<'_>,
        encoded: &JString<'_>,
        append: bool,
    ) -> jni::errors::Result<Option<String>> {
        let object = bridge::call_bridge_method(
            env,
            activity,
            &bridge::methods::WRITE_SAF_FILE_CHUNK,
            &[
                JValue::Object(uri.as_ref()),
                JValue::Object(relative.as_ref()),
                JValue::Object(encoded.as_ref()),
                JValue::Bool(append),
            ],
        )?
        .l()?;
        if object.is_null() {
            return Ok(None);
        }
        Ok(Some(JString::cast_local(env, object)?.try_to_string(env)?))
    }

    fn state_path(dir: &Path, slot_id: u64) -> PathBuf {
        dir.join(format!("{slot_id}.state"))
    }
}

/// Per-crossing SAF chunk: 64KB of raw bytes per JNI call.
/// Must equal `SafRepository.SAF_CHUNK_BYTES` in Kotlin.
const SAF_CHUNK_BYTES: usize = 64 * 1024;

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

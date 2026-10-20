package io.github.chalharu.nerust

import android.content.Context
import android.net.Uri
import android.util.Base64
import android.util.Log
import androidx.documentfile.provider.DocumentFile
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors

/**
 * SAF document I/O behind the JNI bridge.
 *
 * Extracted from MainActivity. All methods are synchronous and run on the
 * calling thread: the Rust bridge invokes them from session/persistence
 * threads, never the UI thread. The [ioExecutor] backs the `*Async`
 * variants for any future UI-thread caller; the sync JNI path stays
 * synchronous because JNI requires synchronous returns.
 *
 * Large files never cross JNI whole: [readSafFileChunkBytes]/[writeSafFileChunkBytes]
 * bound every crossing to 64KB of raw `byte[]` (see `SAF_CHUNK_BYTES` on the
 * Rust side; the Base64 chunk methods stay for APKs predating the `byte[]`
 * bridge). The legacy whole-file methods stay for even older APKs; the Rust
 * side prefers `byte[]`, then Base64 chunks, then whole-file, falling back
 * on `NoSuchMethodError`.
 */
class SafRepository(private val context: Context) {

    private val ioExecutor: ExecutorService = Executors.newSingleThreadExecutor()

    fun readSafFile(treeUri: String, relativePath: String): String? {
        val file = resolveSafDocument(treeUri, relativePath, create = false) ?: return null
        return context.contentResolver.openInputStream(file.uri)?.use { input ->
            Base64.encodeToString(input.readBytes(), Base64.NO_WRAP)
        }
    }

    fun writeSafFile(treeUri: String, relativePath: String, encoded: String): String {
        val file = resolveSafDocument(treeUri, relativePath, create = true) ?: return "error"
        val bytes = Base64.decode(encoded, Base64.NO_WRAP)
        context.contentResolver.openOutputStream(file.uri, "rwt")?.use { it.write(bytes) }
            ?: return "error"
        return "ok"
    }

    fun deleteSafFile(treeUri: String, relativePath: String): String {
        val file = resolveSafDocument(treeUri, relativePath, create = false) ?: return "missing"
        return if (file.delete()) "ok" else "error"
    }

    fun listSafFiles(treeUri: String, relativePath: String): String {
        val directory =
            resolveSafDirectory(treeUri, relativePath, create = false)
                ?: return "[]"
        return org.json.JSONArray(directory.listFiles().mapNotNull(DocumentFile::getName)).toString()
    }

    /**
     * Size probe for the chunked read: bytes, or -1 when the document does
     * not exist. New bridge (v2); the Rust side falls back to whole-file
     * I/O when this method is absent.
     */
    fun getSafFileSize(treeUri: String, relativePath: String): Long {
        val file = resolveSafDocument(treeUri, relativePath, create = false) ?: return -1L
        return file.length().takeIf { it >= 0 } ?: -1L
    }

    /**
     * One chunk read at [offset] ([length] bytes max), Base64-encoded, or
     * null when the document does not exist.
     */
    fun readSafFileChunk(
        treeUri: String,
        relativePath: String,
        offset: Long,
        length: Long,
    ): String? {
        val file = resolveSafDocument(treeUri, relativePath, create = false) ?: return null
        context.contentResolver.openInputStream(file.uri)?.use { input ->
            var remaining = offset.coerceAtLeast(0L)
            while (remaining > 0) {
                val skipped = input.skip(remaining)
                if (skipped <= 0) return null
                remaining -= skipped
            }
            val capped = length.coerceIn(0L, SAF_CHUNK_BYTES.toLong()).toInt()
            val buffer = ByteArray(capped)
            var filled = 0
            while (filled < capped) {
                val read = input.read(buffer, filled, capped - filled)
                if (read < 0) break
                filled += read
            }
            return Base64.encodeToString(buffer.copyOf(filled), Base64.NO_WRAP)
        }
        return null
    }

    /**
     * One chunk write. [append] selects `"wa"` (append) over `"rwt"`
     * (overwrite for the first chunk). Returns "ok" or "error" like the
     * legacy whole-file method.
     */
    fun writeSafFileChunk(
        treeUri: String,
        relativePath: String,
        encoded: String,
        append: Boolean,
    ): String {
        val file = resolveSafDocument(treeUri, relativePath, create = true) ?: return "error"
        val bytes = try {
            if (encoded.isEmpty()) ByteArray(0) else Base64.decode(encoded, Base64.NO_WRAP)
        } catch (error: IllegalArgumentException) {
            Log.w(TAG, "writeSafFileChunk: bad Base64", error)
            return "error"
        }
        val mode = if (append) "wa" else "rwt"
        context.contentResolver.openOutputStream(file.uri, mode)?.use { it.write(bytes) }
            ?: return "error"
        return "ok"
    }

    /**
     * One chunk read at [offset] ([length] bytes max) as a raw `byte[]`,
     * or null when the document does not exist. No Base64 anywhere: this
     * is the preferred chunk bridge (v3); the Base64 variant stays for
     * older Rust builds.
     */
    fun readSafFileChunkBytes(
        treeUri: String,
        relativePath: String,
        offset: Long,
        length: Long,
    ): ByteArray? {
        val file = resolveSafDocument(treeUri, relativePath, create = false) ?: return null
        context.contentResolver.openInputStream(file.uri)?.use { input ->
            var remaining = offset.coerceAtLeast(0L)
            while (remaining > 0) {
                val skipped = input.skip(remaining)
                if (skipped <= 0) return null
                remaining -= skipped
            }
            val capped = length.coerceIn(0L, SAF_CHUNK_BYTES.toLong()).toInt()
            val buffer = ByteArray(capped)
            var filled = 0
            while (filled < capped) {
                val read = input.read(buffer, filled, capped - filled)
                if (read < 0) break
                filled += read
            }
            return buffer.copyOf(filled)
        }
        return null
    }

    /**
     * One chunk write of raw [bytes]. [append] selects `"wa"` (append) over
     * `"rwt"` (overwrite for the first chunk). Preferred chunk bridge (v3).
     */
    fun writeSafFileChunkBytes(
        treeUri: String,
        relativePath: String,
        bytes: ByteArray,
        append: Boolean,
    ): String {
        val file = resolveSafDocument(treeUri, relativePath, create = true) ?: return "error"
        val mode = if (append) "wa" else "rwt"
        context.contentResolver.openOutputStream(file.uri, mode)?.use { it.write(bytes) }
            ?: return "error"
        return "ok"
    }

    /**
     * Batched name+size listing (`[{"name": "...", "size": 123}]`) so the
     * Rust `scan` needs no per-entry size probe (no N+1). Sizes are
     * best-effort (`length()` may return -1); unknown sizes encode as -1
     * and Rust treats them as absent.
     */
    fun listSafFilesWithSizes(treeUri: String, relativePath: String): String {
        val directory =
            resolveSafDirectory(treeUri, relativePath, create = false)
                ?: return "[]"
        val array = org.json.JSONArray()
        for (file in directory.listFiles()) {
            val name = file.name ?: continue
            array.put(org.json.JSONObject().put("name", name).put("size", file.length()))
        }
        return array.toString()
    }

    fun readSafFileAsync(treeUri: String, relativePath: String, callback: (String?) -> Unit) {
        ioExecutor.execute { callback(readSafFile(treeUri, relativePath)) }
    }

    fun writeSafFileAsync(
        treeUri: String,
        relativePath: String,
        encoded: String,
        callback: (String) -> Unit,
    ) {
        ioExecutor.execute { callback(writeSafFile(treeUri, relativePath, encoded)) }
    }

    fun listSafFilesAsync(treeUri: String, relativePath: String, callback: (String) -> Unit) {
        ioExecutor.execute { callback(listSafFiles(treeUri, relativePath)) }
    }

    private fun resolveSafDocument(
        treeUri: String,
        relativePath: String,
        create: Boolean,
    ): DocumentFile? {
        val segments = relativePath.split('/').filter(String::isNotBlank)
        val fileName = segments.lastOrNull() ?: return null
        val directory =
            resolveSafDirectory(treeUri, segments.dropLast(1).joinToString("/"), create)
                ?: return null
        return directory.findFile(fileName)
            ?: if (create) directory.createFile("application/octet-stream", fileName) else null
    }

    private fun resolveSafDirectory(
        treeUri: String,
        relativePath: String,
        create: Boolean,
    ): DocumentFile? {
        var current = DocumentFile.fromTreeUri(context, Uri.parse(treeUri)) ?: return null
        for (segment in relativePath.split('/').filter(String::isNotBlank)) {
            current = resolveSafChildDirectory(current, segment, create) ?: return null
        }
        return current
    }

    private fun resolveSafChildDirectory(
        parent: DocumentFile,
        name: String,
        create: Boolean,
    ): DocumentFile? =
        parent.findFile(name)?.takeIf(DocumentFile::isDirectory)
            ?: if (create) parent.createDirectory(name) else null

    companion object {
        private const val TAG = "Nerust"
        const val SAF_CHUNK_BYTES = 64 * 1024
    }
}

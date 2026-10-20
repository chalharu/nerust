package io.github.chalharu.nerust

import android.content.Context
import android.net.Uri
import android.util.Base64
import android.util.Log
import androidx.documentfile.provider.DocumentFile

/** SAF document I/O behind the JNI bridge. All methods run synchronously on the calling thread.
 * Large files cross in 64KB `byte[]` chunks; legacy whole-file and Base64 methods stay for old APKs. */
class SafRepository(private val context: Context) {

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

    /** Size probe for the chunked read: bytes, or -1 when the document does not exist. */
    fun getSafFileSize(treeUri: String, relativePath: String): Long {
        val file = resolveSafDocument(treeUri, relativePath, create = false) ?: return -1L
        return file.length().takeIf { it >= 0 } ?: -1L
    }

    /** One Base64 chunk read at [offset] ([length] bytes max); null when the document is missing. */
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

    /** One chunk write; [append] selects append over overwrite. Returns "ok" or "error". */
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

    /** One chunk read at [offset] ([length] bytes max) as raw `byte[]`; null when missing. */
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

    /** One chunk write of raw [bytes]; [append] selects append over overwrite. */
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

    /** Batched name+size listing so Rust `scan` needs no per-entry probe. Unknown sizes encode as -1. */
    fun listSafFilesWithSizes(treeUri: String, relativePath: String): String {
        val directory =
            resolveSafDirectory(treeUri, relativePath, create = false)
                ?: return "[]"
        val array = org.json.JSONArray()
        for (file in directory.listFiles()) {
            val name = file.name ?: continue
            val size = file.length().takeIf { it >= 0 } ?: -1L
            array.put(org.json.JSONObject().put("name", name).put("size", size))
        }
        return array.toString()
    }

    /** Server-side streaming session: one `openInputStream`, sequential reads.
     * Rust pulls chunks until an empty array; the session closes explicitly. */
    fun openSafReadSession(treeUri: String, relativePath: String): Long {
        val file = resolveSafDocument(treeUri, relativePath, create = false) ?: return -1L
        val stream =
            try {
                context.contentResolver.openInputStream(file.uri)
            } catch (error: Exception) {
                Log.w(TAG, "openSafReadSession: open failed", error)
                null
            } ?: return -1L
        synchronized(readSessions) {
            val id = nextReadSessionId++
            readSessions[id] = stream
            return id
        }
    }

    /** One sequential chunk from an open session; empty marks end of stream. */
    fun readSafReadSession(sessionId: Long, maxBytes: Int): ByteArray {
        val stream =
            synchronized(readSessions) { readSessions[sessionId] }
                ?: return ByteArray(0)
        val capped = maxBytes.coerceIn(1, SAF_CHUNK_BYTES)
        return try {
            val buffer = ByteArray(capped)
            var filled = 0
            while (filled < capped) {
                val read = stream.read(buffer, filled, capped - filled)
                if (read < 0) break
                filled += read
            }
            buffer.copyOf(filled)
        } catch (error: Exception) {
            Log.w(TAG, "readSafReadSession: read failed", error)
            ByteArray(0)
        }
    }

    /** Close a server-side read session; a missing session is a no-op. */
    fun closeSafReadSession(sessionId: Long) {
        val stream =
            synchronized(readSessions) { readSessions.remove(sessionId) }
                ?: return
        try {
            stream.close()
        } catch (error: Exception) {
            Log.w(TAG, "closeSafReadSession: close failed", error)
        }
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

    private val readSessions = mutableMapOf<Long, java.io.InputStream>()
    private var nextReadSessionId = 1L
}

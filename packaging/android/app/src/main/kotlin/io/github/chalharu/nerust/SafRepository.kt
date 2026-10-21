package io.github.chalharu.nerust

import android.content.Context
import android.net.Uri
import android.util.Log
import androidx.documentfile.provider.DocumentFile

/** SAF document I/O behind the JNI bridge. All methods run synchronously on the calling thread.
 * Rust and Kotlin are bundled: one raw-byte contract, no legacy transports. */
class SafRepository(private val context: Context) : AutoCloseable {

    fun deleteSafFile(treeUri: String, relativePath: String): String {
        val file = resolveSafDocument(treeUri, relativePath, create = false) ?: return "missing"
        return if (file.delete()) "ok" else "error"
    }

    /** One chunk write of raw [bytes]; [append] selects append over overwrite. */
    fun writeSafFileChunkBytes(
        treeUri: String,
        relativePath: String,
        bytes: ByteArray,
        append: Boolean,
    ): Boolean {
        require(bytes.size <= SAF_CHUNK_BYTES)
        val file = resolveSafDocument(treeUri, relativePath, create = true) ?: return false
        val mode = if (append) "wa" else "rwt"
        context.contentResolver.openOutputStream(file.uri, mode)?.use { it.write(bytes) }
            ?: return false
        return true
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

    /** -1 = missing document, 0 = open error, positive = session ID. */
    fun openSafReadSession(treeUri: String, relativePath: String): Long {
        return try {
            val file = resolveSafDocument(treeUri, relativePath, create = false) ?: return -1L
            val stream = context.contentResolver.openInputStream(file.uri) ?: return 0L
            readSessions.open(stream)
        } catch (error: Exception) {
            Log.w(TAG, "openSafReadSession: open failed", error)
            0L
        }
    }

    /** Nonempty = data, empty = EOF, null = invalid session or read error. */
    fun readSafReadSession(sessionId: Long, maxBytes: Int): ByteArray? =
        readSessions.read(sessionId, maxBytes)

    /** Close a server-side read session; a missing session is a no-op. */
    fun closeSafReadSession(sessionId: Long) {
        readSessions.close(sessionId)
    }

    override fun close() = readSessions.close()

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
    }

    private val readSessions = SafReadSessions()
}

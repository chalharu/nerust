package io.github.chalharu.nerust

import android.content.Context
import android.net.Uri
import android.os.ParcelFileDescriptor
import android.provider.OpenableColumns
import android.util.Log
import androidx.documentfile.provider.DocumentFile
import java.io.FilterOutputStream
import java.util.UUID
import java.util.concurrent.atomic.AtomicLong
import org.json.JSONObject

/** SAF document I/O behind the JNI bridge. All methods run synchronously on the calling thread.
 * Rust and Kotlin are bundled: one raw-byte contract, no legacy transports. */
class SafRepository(private val context: Context) : AutoCloseable {

    fun deleteSafFile(treeUri: String, relativePath: String): String {
        val (directory, name) = parent(treeUri, relativePath, false) ?: return "missing"
        return if (publish(directory, name, null, 0)) "ok" else "error"
    }

    /** Immutable generations are the fallback for providers without atomic replace/rename.
     * No committed document is truncated. Incomplete generations/markers are invisible. */
    fun openSafWriteSession(treeUri: String, relativePath: String): Long {
        return try {
            val (directory, name) = parent(treeUri, relativePath, true) ?: return 0L
            val data = directory.createFile("application/octet-stream", "$PREFIX${UUID.randomUUID()}.data")
                ?: return 0L
            val output = try { durableOutput(data.uri) }
                catch (error: Exception) { data.delete(); throw error }
            if (output == null) { data.delete(); return 0L }
            var size = 0L
            val counting = object : FilterOutputStream(output) {
                override fun write(value: Int) { out.write(value); size++ }
                override fun write(bytes: ByteArray, offset: Int, length: Int) {
                    out.write(bytes, offset, length)
                    size += length
                }
            }
            writeSessions.open(object : SafWriteSessions.Transaction {
                override val stream = counting
                override fun commit(): Boolean = publish(directory, name, requireNotNull(data.name), size)
                override fun abort() { data.delete() }
            })
        } catch (error: Exception) {
            Log.w(TAG, "SAF staging failed", error)
            0L
        }
    }

    fun writeSafWriteSession(id: Long, bytes: ByteArray): Boolean = writeSessions.write(id, bytes)
    fun closeSafWriteSession(id: Long, commit: Boolean): Boolean = writeSessions.finish(id, commit)

    /** Batched name+size listing so Rust `scan` needs no per-entry probe. Unknown sizes encode as -1. */
    fun listSafFilesWithSizes(treeUri: String, relativePath: String): String {
        val directory =
            resolveSafDirectory(treeUri, relativePath, create = false)
                ?: return "[]"
        val array = org.json.JSONArray()
        val files = directory.listFiles()
        val visible = files.filter { it.name?.startsWith(PREFIX) == false }
            .associateBy { requireNotNull(it.name) }.toMutableMap()
        val committed = generations(files)
        for ((name, generation) in committed) {
            if (generation.data == null) visible.remove(name) else visible[name] = generation.data
        }
        // Sidecar bytes ride the same listing so one generations pass serves
        // the whole Rust scan instead of re-resolving per slot (NER-310).
        val byName = files.associateBy { it.name }
        for ((name, file) in visible) {
            val size = committed[name]?.size ?: knownSize(file) ?: -1L
            val entry = org.json.JSONObject().put("name", name).put("size", size)
            if (name.endsWith(".state")) {
                val sidecar = byName[name.removeSuffix(".state") + ".summary"]
                val bytes = try {
                    sidecar?.uri?.let { uri ->
                        context.contentResolver.openInputStream(uri)?.use { it.readBytes() }
                    }
                } catch (error: Exception) {
                    Log.w(TAG, "listSafFilesWithSizes: sidecar read failed", error)
                    null
                }
                if (bytes != null) {
                    entry.put("summary", android.util.Base64.encodeToString(bytes, android.util.Base64.NO_WRAP))
                }
            }
            array.put(entry)
        }
        // Amortized recovery scan: staged data without a commit marker is a
        // process-death orphan once past the grace window. In-flight stages
        // have fresh or unknown mtimes and are never touched.
        reclaimOrphanedStages(directory, files, committed.values.mapNotNull { it.data?.name }.toSet())
        return array.toString()
    }

    /** -1 = missing document, 0 = open error, positive = session ID. */
    fun openSafReadSession(treeUri: String, relativePath: String): Long {
        return try {
            val (directory, name) = parent(treeUri, relativePath, false) ?: return -1L
            val files = directory.listFiles()
            val generation = generations(files)[name]
            val file = if (generation != null) generation.data else files.find { it.name == name }
            if (file == null) return -1L
            val stream = context.contentResolver.openInputStream(file.uri) ?: return 0L
            readSessions.open(if (generation == null) stream else SafVerifiedInputStream(stream, generation.size))
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

    override fun close() {
        readSessions.close()
        writeSessions.close()
    }

    /** Delete same-name markers older than a fresh commit, plus their data files. */
    private fun sweepSupersededMarkers(directory: DocumentFile, name: String, order: String) {
        for (marker in directory.listFiles()) {
            val markerName = marker.name
            if (!isCommitMarker(markerName)) continue
            if (markerName == commitMarkerName(order)) continue
            try {
                val raw = readCommitMarker(marker) ?: continue
                val record = JSONObject(raw)
                if (isSupersededMarker(record.optString("name"), record.optString("order"), name, order)) {
                    record.optString("data").takeIf { it.startsWith(PREFIX) }?.let { data ->
                        runCatching { directory.findFile(data)?.delete() }
                    }
                    marker.delete()
                }
            } catch (error: Exception) {
                Log.w(TAG, "Ignoring unsweepable SAF marker", error)
            }
        }
    }

    /**
     * Amortized orphan recovery: delete unreferenced staged data past the
     * grace window, capped per listing. A delete failure stays for the next
     * listing; an in-flight stage is never a candidate.
     */
    private fun reclaimOrphanedStages(
        directory: DocumentFile,
        files: Array<DocumentFile>,
        referenced: Set<String>,
    ) {
        val now = System.currentTimeMillis()
        for (name in selectOrphanedStages(files.mapNotNull { file ->
            val fileName = file.name
            if (fileName != null && isStagedData(fileName)) fileName to file.lastModified() else null
        }.toMap(), referenced, now)) {
            runCatching { directory.findFile(name)?.delete() }
        }
    }

    private fun parent(
        treeUri: String,
        relativePath: String,
        create: Boolean,
    ): Pair<DocumentFile, String>? {
        val segments = relativePath.split('/')
        if (segments.any { it.isBlank() || it == "." || it == ".." || it.contains('\\') }) return null
        val fileName = segments.lastOrNull() ?: return null
        if (fileName.startsWith(PREFIX)) return null
        val directory =
            resolveSafDirectory(treeUri, segments.dropLast(1).joinToString("/"), create)
                ?: return null
        return directory to fileName
    }

    private data class Generation(val order: String, val data: DocumentFile?, val size: Long)

    private fun knownSize(file: DocumentFile): Long? =
        runCatching {
            context.contentResolver.query(file.uri, arrayOf(OpenableColumns.SIZE), null, null, null)?.use {
                val column = it.getColumnIndex(OpenableColumns.SIZE)
                if (!it.moveToFirst() || column < 0 || it.isNull(column)) null
                else it.getLong(column).takeIf { size -> size >= 0 }
            }
        }.getOrNull()

    private fun generations(files: Array<DocumentFile>): Map<String, Generation> {
        val byName = files.associateBy { it.name }
        val result = mutableMapOf<String, Generation>()
        for (marker in commitMarkers(files)) {
            try {
                val raw = readCommitMarker(marker) ?: continue
                val (name, generation) = parseGeneration(raw, byName) ?: continue
                if (result[name]?.order?.let { it >= generation.order } != true) {
                    result[name] = generation
                }
            } catch (error: Exception) {
                Log.w(TAG, "Ignoring incomplete SAF commit", error)
            }
        }
        return result
    }

    private fun commitMarkers(files: Array<DocumentFile>): List<DocumentFile> =
        files.filter { isCommitMarker(it.name) }

    private fun readCommitMarker(marker: DocumentFile): String? {
        val stream = context.contentResolver.openInputStream(marker.uri) ?: return null
        return stream.use {
            val bytes = ByteArray(4097)
            var used = 0
            while (used < bytes.size) {
                val count = it.read(bytes, used, bytes.size - used)
                if (count < 0) break
                if (count == 0) throw java.io.IOException("zero marker read")
                used += count
            }
            if (used > 4096) return null
            String(bytes, 0, used, Charsets.UTF_8)
        }
    }

    private fun parseGeneration(raw: String, byName: Map<String?, DocumentFile>): Pair<String, Generation>? {
        val record = JSONObject(raw)
        if (record.optInt("version", -1) != 1 || !record.optBoolean("complete", false)) return null
        if (!record.has("data") || record.optLong("size", -1) < 0) return null
        val name = record.getString("name")
        val order = record.getString("order")
        val size = record.getLong("size")
        val data = if (record.isNull("data")) null else {
            val file = byName[record.getString("data")] ?: return null
            // Null SIZE is unknown, not an empty file. Verify at EOF on read.
            // A known-size mismatch skips this marker so the previous
            // generation keeps serving; the corrupt marker is swept on
            // the next successful publish.
            val length = knownSize(file)
            if (length != null && length != size) {
                Log.w(TAG, "SAF marker size mismatch; keeping previous generation")
                return null
            }
            file
        }
        return name to Generation(order, data, size)
    }

    private fun publish(directory: DocumentFile, name: String, data: String?, size: Long): Boolean {
        val existing = directory.listFiles().mapNotNull {
            it.name?.takeIf { name -> isCommitMarker(name) }
                ?.removePrefix(PREFIX)?.take(19)?.toLongOrNull()
        }.maxOrNull() ?: 0L
        val order = generationClock.updateAndGet { maxOf(it + 1, existing + 1, System.currentTimeMillis()) }
            .toString().padStart(19, '0') + "-" + UUID.randomUUID()
        // Octet-stream avoids providers appending a MIME-derived .json extension.
        val marker = directory.createFile("application/octet-stream", commitMarkerName(order)) ?: return false
        return try {
            val record = JSONObject().put("version", 1).put("name", name).put("order", order)
                .put("data", data ?: JSONObject.NULL).put("size", size).put("complete", true).toString()
            val output = durableOutput(marker.uri)
                ?: throw java.io.IOException("missing commit stream")
            output.use { it.write(record.toByteArray(Charsets.UTF_8)); it.flush() }
            sweepSupersededMarkers(directory, name, order)
            true
        } catch (error: Exception) {
            abortPublish(directory, marker, data)
            Log.w(TAG, "SAF commit failed; previous generation retained", error)
            false
        }
    }

    /** Remove a failed commit's marker and its staged data file. */
    private fun abortPublish(directory: DocumentFile, marker: DocumentFile, data: String?) {
        runCatching { marker.delete() }
        // The staged data belongs to this failed commit only; drop it too.
        data?.takeIf { it.startsWith(PREFIX) }?.let { staged ->
            runCatching { directory.findFile(staged)?.delete() }
        }
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

    /** A provider that cannot durably sync must fail without touching the previous save. */
    private fun durableOutput(uri: Uri): java.io.OutputStream? {
        val descriptor = context.contentResolver.openFileDescriptor(uri, "w") ?: return null
        return object : ParcelFileDescriptor.AutoCloseOutputStream(descriptor) {
            override fun close() {
                try { flush(); fd.sync() } finally { super.close() }
            }
        }
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
        private const val PREFIX = ".nerust-generation-"
        private const val COMMIT_SUFFIX = ".commit"
        private const val MAX_RECLAIM_PER_LISTING = 4
        private const val ORPHAN_GRACE_MS = 3_600_000L
        private val generationClock = AtomicLong()

        /** Commit-marker file name for one publish order. */
        internal fun commitMarkerName(order: String) = "$PREFIX$order$COMMIT_SUFFIX"

        /** True for files this repository manages as commit markers. */
        internal fun isCommitMarker(name: String?) =
            name != null && name.startsWith(PREFIX) && name.endsWith(COMMIT_SUFFIX)

        /** True for staged data files awaiting (or orphaned by) a commit marker. */
        internal fun isStagedData(name: String) =
            name.startsWith(PREFIX) && name.endsWith(".data")

        /**
         * Orphaned staged data: unreferenced by any live marker and older than
         * the grace window. Unknown mtimes (0) count as fresh, never stale.
         * Stable name order keeps successive capped sweeps deterministic.
         */
        internal fun selectOrphanedStages(
            dataFiles: Map<String, Long>,
            referenced: Set<String>,
            nowMs: Long,
        ): List<String> =
            dataFiles
                .filter { (name, _) -> name !in referenced }
                .filter { (_, modified) -> modified > 0 && nowMs - modified > ORPHAN_GRACE_MS }
                .keys
                .sorted()
                .take(MAX_RECLAIM_PER_LISTING)

        /** Sweep predicate: same name with a strictly older order. Orders sort lexicographically. */
        internal fun isSupersededMarker(
            recordName: String,
            recordOrder: String,
            name: String,
            order: String,
        ): Boolean = recordName == name && recordOrder < order
    }

    private val readSessions = SafReadSessions()
    private val writeSessions = SafWriteSessions()
}


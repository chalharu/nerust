package io.github.chalharu.nerust

import android.content.Context
import android.net.Uri
import android.os.ParcelFileDescriptor
import android.provider.DocumentsContract
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
        // A bad tree is a configuration failure, not an empty listing: throwing
        // surfaces it as a scan error instead of a silent empty (NER-322).
        // A missing child directory is genuinely empty and stays "[]".
        DocumentFile.fromTreeUri(context, Uri.parse(treeUri))
            ?: throw IllegalArgumentException("unresolvable SAF tree: $treeUri")
        // DocumentFile.listFiles() swallows provider failures to an empty
        // array, which a vacuous survey would certify complete and clear the
        // visible slots over. Probe the tree root first: an unreadable root
        // is a scan error (the Rust side keeps the previous list), never an
        // empty one (NER-321/322).
        check(verifiedTreeRoot(treeUri)) { "unreadable SAF listing: $treeUri" }
        val directory =
            resolveSafDirectory(treeUri, relativePath, create = false)
                ?: return "[]"
        val array = org.json.JSONArray()
        val files = directory.listFiles()
        val visible = files.filter { it.name?.startsWith(PREFIX) == false }
            .associateBy { requireNotNull(it.name) }.toMutableMap()
        val survey = surveyGenerations(files)
        val committed = survey.committed
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
                // Logical resolution first: generation-tracked sidecars have
                // no stable physical name, so the committed map (not the raw
                // listing) tells which staged file holds the current bytes.
                // A tombstoned sidecar resolves to nothing and falls through
                // to the archive, like a missing one (NER-310).
                val base = name.removeSuffix(".state")
                val sidecar = batchSidecarFile(base, { committed[it]?.data }, byName)
                val bytes = try {
                    sidecar?.uri?.let { uri ->
                        context.contentResolver.openInputStream(uri)?.use { stream ->
                            readCapped(stream, SAF_SUMMARY_BYTES + 1)
                        }?.takeIf { it.size <= SAF_SUMMARY_BYTES }
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
        // have fresh or unknown mtimes and are never touched. An incomplete
        // survey skips reclamation: a missed marker must not orphan live data.
        if (!survey.complete) {
            Log.w(TAG, "skipping orphan reclaim: incomplete generation survey")
        } else {
            reclaimOrphanedStages(
                directory,
                files,
                survey.allReferenced,
                survey.complete,
            )
        }
        return array.toString()
    }

    /** -1 = missing document, 0 = open error, positive = session ID.
     * NER-321: a name with no generation and no physical file is missing
     * only after a COMPLETE survey. An incomplete survey (an unreadable
     * commit marker) returns an error instead: collapsing it to missing
     * would let the Rust side treat live data as gone and run the
     * delete/tombstone/GC path over it. */
    fun openSafReadSession(treeUri: String, relativePath: String): Long {
        return try {
            // Same probe-first discipline as the listing: a provider-wide
            // outage must surface as 0 (error, never delete), not -1
            // (missing, which runs the delete/tombstone/GC path) (NER-321).
            if (!verifiedTreeRoot(treeUri)) return 0L
            val (directory, name) = parent(treeUri, relativePath, false) ?: return -1L
            val files = directory.listFiles()
            val survey = surveyGenerations(files)
            val generation = survey.committed[name]
            if (generation != null) {
                // A parsed generation speaks for this name even when other
                // markers failed: a tombstone is confirmed-absent, otherwise
                // the staged data serves under its committed size.
                val file = generation.data ?: return -1L
                val stream = context.contentResolver.openInputStream(file.uri) ?: return 0L
                readSessions.open(SafVerifiedInputStream(stream, generation.size))
            } else {
                val file = files.find { it.name == name }
                    ?: return if (confirmedAbsent(survey.complete, false)) -1L else 0L
                val stream = context.contentResolver.openInputStream(file.uri) ?: return 0L
                readSessions.open(stream)
            }
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
        surveyComplete: Boolean,
    ) {
        val now = System.currentTimeMillis()
        for (name in selectOrphanedStages(files.mapNotNull { file ->
            val fileName = file.name
            if (fileName != null && isStagedData(fileName)) fileName to file.lastModified() else null
        }.toMap(), referenced, now, surveyComplete)) {
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

    private fun readCapped(stream: java.io.InputStream, limit: Int): ByteArray =
        readCappedBytes(stream, limit)

    private fun knownSize(file: DocumentFile): Long? =
        runCatching {
            context.contentResolver.query(file.uri, arrayOf(OpenableColumns.SIZE), null, null, null)?.use {
                val column = it.getColumnIndex(OpenableColumns.SIZE)
                if (!it.moveToFirst() || column < 0 || it.isNull(column)) null
                else it.getLong(column).takeIf { size -> size >= 0 }
            }
        }.getOrNull()

    /**
     * Generations survey: the display map plus whether every marker resolved.
     * A single unreadable marker makes the survey incomplete; destructive
     * reclamation must not run on an incomplete survey (NER-311).
     * [allReferenced] covers every successfully parsed generation, not just
     * the latest per name: GC must not orphan data an older unswept marker
     * still points at, even when a newer generation already serves.
     */
    private fun surveyGenerations(files: Array<DocumentFile>): GenerationSurvey {
        val byName = files.associateBy { it.name }
        val result = mutableMapOf<String, Generation>()
        val referenced = mutableSetOf<String>()
        var complete = true
        for (marker in commitMarkers(files)) {
            try {
                val raw = readCommitMarker(marker) ?: run { complete = false; continue }
                val (name, generation) = parseGeneration(raw, byName) ?: run {
                    complete = false
                    continue
                }
                generation.data?.name?.let { referenced += it }
                if (result[name]?.order?.let { it >= generation.order } != true) {
                    result[name] = generation
                }
            } catch (error: Exception) {
                Log.w(TAG, "Ignoring incomplete SAF commit", error)
                complete = false
            }
        }
        return GenerationSurvey(result, complete, referenced)
    }

    private data class GenerationSurvey(
        val committed: Map<String, Generation>,
        val complete: Boolean,
        val allReferenced: Set<String> = emptySet(),
    )

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

    /** Probe-first listing health: DocumentFile list/find calls swallow
     * provider failures (empty array / null), so a vacuous result cannot
     * prove absence. Query the tree root document directly: it throws on a
     * provider-wide outage. Non-tree URIs (single-document and legacy
     * providers) keep the historical behavior since the probe is a
     * DocumentsContract concept that must not fail them (NER-321). */
    private fun verifiedTreeRoot(treeUri: String): Boolean {
        val parsed = Uri.parse(treeUri)
        if (parsed.pathSegments.firstOrNull() != "tree") return true
        val rootId = try {
            DocumentsContract.getTreeDocumentId(parsed)
        } catch (error: Exception) {
            Log.w(TAG, "unreadable SAF tree id", error)
            return false
        }
        val probeUri = DocumentsContract.buildDocumentUriUsingTree(parsed, rootId)
        return runCatching {
            context.contentResolver.query(
                probeUri,
                arrayOf(DocumentsContract.Document.COLUMN_DOCUMENT_ID),
                null,
                null,
                null,
            )?.use { it.moveToFirst() } == true
        }.getOrDefault(false)
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
         * Batch sidecar resolution for one slot: the committed generation
         * wins over the physical listing, and a tombstoned or absent sidecar
         * resolves to nothing so the scan falls back to the archive.
         * Generation-tracked sidecars have no stable physical name, so the
         * raw listing alone would always miss them (NER-310).
         */
        internal fun batchSidecarFile(
            base: String,
            committedData: (String) -> DocumentFile?,
            physical: Map<String?, DocumentFile?>,
        ): DocumentFile? =
            committedData("$base.summary") ?: physical["$base.summary"]

        /**
         * Orphaned staged data: unreferenced by any live marker and older than
         * the grace window. Unknown mtimes (0) count as fresh, never stale.
         * Stable name order keeps successive capped sweeps deterministic.
         * An incomplete survey selects nothing: destructive reclamation needs
         * every marker resolved, so a missed marker never orphans live data.
         */
        internal fun selectOrphanedStages(
            dataFiles: Map<String, Long>,
            referenced: Set<String>,
            nowMs: Long,
            surveyComplete: Boolean = true,
        ): List<String> =
            if (!surveyComplete) {
                emptyList()
            } else {
                dataFiles
                    .filter { (name, _) -> name !in referenced }
                    .filter { (_, modified) -> modified > 0 && nowMs - modified > ORPHAN_GRACE_MS }
                    .keys
                    .sorted()
                    .take(MAX_RECLAIM_PER_LISTING)
            }

        /** Sweep predicate: same name with a strictly older order. Orders sort lexicographically. */
        internal fun isSupersededMarker(
            recordName: String,
            recordOrder: String,
            name: String,
            order: String,
        ): Boolean = recordName == name && recordOrder < order

        /**
         * NER-321 read-outcome gate: a logical document is confirmed-absent
         * only after a COMPLETE generation survey. An incomplete survey (an
         * unreadable commit marker) must surface as an open error, never as
         * missing, so the Rust side keeps its previous state and never runs
         * the delete/tombstone/GC path over live data.
         * Pure decision logic: JVM tests pin the truth table without a device.
         */
        internal fun confirmedAbsent(surveyComplete: Boolean, fileFound: Boolean): Boolean =
            surveyComplete && !fileFound

        /** Scan transfer guard: sidecar bytes above this are skipped, so a huge
         * summary never crosses on the batch path either (NER-312). Pure
         * `java.io`: JVM tests pin the bound without a provider. */
        internal fun readCappedBytes(stream: java.io.InputStream, limit: Int): ByteArray {
            val out = java.io.ByteArrayOutputStream()
            val chunk = ByteArray(8192)
            var remaining = limit
            while (remaining > 0) {
                val count = stream.read(chunk, 0, minOf(chunk.size, remaining))
                if (count < 0) break
                if (count == 0) throw java.io.IOException("zero sidecar read")
                out.write(chunk, 0, count)
                remaining -= count
            }
            return out.toByteArray()
        }
    }

    private val readSessions = SafReadSessions()
    private val writeSessions = SafWriteSessions()
}


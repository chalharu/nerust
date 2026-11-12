package io.github.chalharu.nerust

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.SwipeToDismissBox
import androidx.compose.material3.SwipeToDismissBoxValue
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.material3.rememberSwipeToDismissBoxState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.util.Locale
import org.json.JSONObject

internal const val SLOT_MENU_TAG = "nerust-slot-menu"

/** Drawer-only submenu opener: handled in Kotlin, never crosses the wire. */
internal const val OPEN_SLOTS = "open_slots"

/** One submenu row: slot id plus its save instant (null = unknown stamp).
 * Active marks the session's current slot, occupied means a save exists. */
internal data class SlotEntry(
    val id: Long,
    val savedAtMs: Long?,
    val occupied: Boolean,
    val active: Boolean,
)

/** Parse the Rust slot-list query; malformed payloads show no rows. */
internal fun parseSlotList(raw: String): List<SlotEntry> {
    val root = try {
        JSONObject(raw)
    } catch (_: Exception) {
        return emptyList()
    }
    val active = if (root.isNull("active")) null else root.optLong("active")
    val array = root.optJSONArray("slots") ?: return emptyList()
    val out = mutableListOf<SlotEntry>()
    for (i in 0 until array.length()) {
        val row = array.optJSONObject(i) ?: continue
        if (row.isNull("id")) continue
        val id = row.optLong("id")
        val savedAtMs = if (row.isNull("saved_at_ms")) null else row.optLong("saved_at_ms")
        out += SlotEntry(id, savedAtMs, occupied = true, active = id == active)
    }
    if (active != null && out.none { it.id == active }) {
        out += SlotEntry(active, savedAtMs = null, occupied = false, active = true)
    }
    return out.sortedBy { it.id }
}

/** Local-time stamp for a submenu subtitle; null shows the unknown label. */
internal fun formatSlotTime(millis: Long): String =
    Instant.ofEpochMilli(millis).atZone(ZoneId.systemDefault()).format(SLOT_TIME_FORMAT)

private val SLOT_TIME_FORMAT: DateTimeFormatter =
    DateTimeFormatter.ofPattern("yyyy-MM-dd HH:mm", Locale.US)

/** Bounded refresh: after a slot op the event thread republishes the
 * vault mirror, so re-query off the main thread until the payload moves
 * past the last seen one (3 tries, 100/200/400ms backoff), then publish.
 * Query work never touches the main looper; only the state assign lands
 * there. Stops with the dialog. */
internal class SlotMenuState(
    initialRaw: String,
    initial: List<SlotEntry>,
) {
    var rows: List<SlotEntry> by mutableStateOf(initial)
        private set
    private var lastRaw = initialRaw

    @Volatile
    private var stopped = false

    // Generation counter: overlapping refresh loops (rapid deletes) must
    // not overwrite each other; only the newest poll writes rows.
    @Volatile
    private var generation = 0

    fun stop() {
        stopped = true
    }

    fun refreshAfter(query: () -> String?) {
        poll(query, ++generation, 0)
    }

    private fun poll(
        query: () -> String?,
        gen: Int,
        attempt: Int,
    ) {
        if (stopped || gen != generation || attempt >= SLOT_REFRESH_DELAYS_MS.size) return
        BridgeIo.scheduleIo(SLOT_REFRESH_DELAYS_MS[attempt]) {
            if (stopped || gen != generation) return@scheduleIo
            val raw = try {
                query()
            } catch (_: Exception) {
                null
            }
            BridgeIo.postMain {
                if (stopped || gen != generation) return@postMain
                if (raw != null && raw != lastRaw) {
                    lastRaw = raw
                    rows = parseSlotList(raw)
                } else {
                    poll(query, gen, attempt + 1)
                }
            }
        }
    }
}

private val SLOT_REFRESH_DELAYS_MS = longArrayOf(100L, 200L, 400L)

/** Full-screen slot manager: a settings-grade screen (top bar + list),
 * not a centered popup. Swipe a row or tap the trash icon to delete
 * (both land on the same irreversible-delete confirm); tap selects. */
@Composable
internal fun SlotMenuScreen(
    rows: List<SlotEntry>,
    onSelect: (Long) -> Unit,
    onDelete: (Long) -> Unit,
    onCreate: () -> Unit,
    onDismiss: () -> Unit,
) {
    var pendingDelete by remember { mutableStateOf<SlotEntry?>(null) }
    // Same expressive theme as the settings card; DialogManager hosts bare.
    NerustExpressiveHost {
        Surface(modifier = Modifier.fillMaxSize()) {
            Scaffold(
                topBar = {
                    TopAppBar(
                        title = { Text(text = stringResource(R.string.drawer_slots)) },
                        navigationIcon = {
                            IconButton(onClick = onDismiss) {
                                Icon(
                                    painter = painterResource(id = R.drawable.ic_settings_back),
                                    contentDescription = stringResource(R.string.settings_back),
                                )
                            }
                        },
                        colors =
                            TopAppBarDefaults.topAppBarColors(
                                containerColor = MaterialTheme.colorScheme.surfaceContainer,
                            ),
                    )
                },
            ) { padding ->
                LazyColumn(contentPadding = padding) {
                    if (rows.isEmpty()) {
                        item {
                            Text(
                                text = stringResource(R.string.slot_empty),
                                style = MaterialTheme.typography.bodyMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                                modifier = Modifier.padding(horizontal = 24.dp, vertical = 16.dp),
                            )
                        }
                    }
                    items(rows, key = { it.id }) { row ->
                        SlotDismissableRow(
                            row = row,
                            onSelect = { onSelect(row.id) },
                            onDelete = { pendingDelete = row },
                        )
                        HorizontalDivider()
                    }
                    item { SlotMenuNewRow(onCreate = onCreate) }
                }
            }
        }
    }
    pendingDelete?.let { row ->
        SlotDeleteConfirm(
            row = row,
            onConfirm = {
                pendingDelete = null
                onDelete(row.id)
            },
            onDismiss = { pendingDelete = null },
        )
    }
}

/** One slot row with swipe-to-delete. The swipe only arms the shared
 * confirm (never deletes directly: deletion is irreversible), so the
 * dismiss state always snaps back. */
@Composable
private fun SlotDismissableRow(
    row: SlotEntry,
    onSelect: () -> Unit,
    onDelete: () -> Unit,
) {
    val dismissState =
        rememberSwipeToDismissBoxState(
            confirmValueChange = { value ->
                if (value != SwipeToDismissBoxValue.Settled && row.occupied) onDelete()
                false
            },
        )
    SwipeToDismissBox(
        state = dismissState,
        enableDismissFromStartToEnd = false,
        enableDismissFromEndToStart = row.occupied,
        backgroundContent = {
            if (row.occupied) {
                Box(
                    modifier = Modifier.fillMaxSize(),
                    contentAlignment = Alignment.CenterEnd,
                ) {
                    Icon(
                        painter = painterResource(id = R.drawable.ic_slot_delete),
                        contentDescription = null,
                        tint = MaterialTheme.colorScheme.error,
                        modifier = Modifier.padding(end = 24.dp),
                    )
                }
            }
        },
        content = {
            SlotMenuRow(
                row = row,
                onSelect = onSelect,
                onDelete = onDelete,
            )
        },
    )
}

@Composable
private fun SlotMenuRow(
    row: SlotEntry,
    onSelect: () -> Unit,
    onDelete: () -> Unit,
) {
    val title = stringResource(R.string.slot_title, row.id)
    val subtitle = slotSubtitle(row)
    Row(
        verticalAlignment = Alignment.CenterVertically,
        modifier =
            Modifier
                .fillMaxWidth()
                .clickable(onClick = onSelect)
                .padding(horizontal = 24.dp, vertical = 12.dp),
    ) {
        Column(modifier = Modifier.weight(1f)) {
            Text(text = title, style = MaterialTheme.typography.titleMedium)
            Text(
                text = subtitle,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        if (row.occupied) {
            IconButton(onClick = onDelete) {
                Icon(
                    painter = painterResource(id = R.drawable.ic_slot_delete),
                    contentDescription = stringResource(R.string.slot_delete),
                    tint = MaterialTheme.colorScheme.error,
                )
            }
        }
    }
}

@Composable
private fun slotSubtitle(row: SlotEntry): String {
    val time =
        if (row.savedAtMs != null) {
            formatSlotTime(row.savedAtMs)
        } else {
            stringResource(if (row.occupied) R.string.slot_unknown_time else R.string.slot_empty)
        }
    return if (row.active) {
        "$time · ${stringResource(R.string.slot_active)}"
    } else {
        time
    }
}

@Composable
private fun SlotMenuNewRow(onCreate: () -> Unit) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        modifier =
            Modifier
                .fillMaxWidth()
                .clickable(onClick = onCreate)
                .padding(horizontal = 24.dp, vertical = 12.dp),
    ) {
        Column(modifier = Modifier.weight(1f)) {
            Text(text = stringResource(R.string.slot_new), style = MaterialTheme.typography.titleMedium)
            Text(
                text = stringResource(R.string.slot_new_hint),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

@Composable
private fun SlotDeleteConfirm(
    row: SlotEntry,
    onConfirm: () -> Unit,
    onDismiss: () -> Unit,
) {
    val title = stringResource(R.string.slot_title, row.id)
    val subtitle = slotSubtitle(row)
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(text = stringResource(R.string.slot_delete_title, title)) },
        text = { Text(text = stringResource(R.string.slot_delete_message, title, subtitle)) },
        confirmButton = {
            TextButton(onClick = onConfirm) {
                Text(
                    text = stringResource(R.string.slot_delete),
                    color = MaterialTheme.colorScheme.error,
                )
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) {
                Text(text = stringResource(R.string.settings_cancel))
            }
        },
    )
}

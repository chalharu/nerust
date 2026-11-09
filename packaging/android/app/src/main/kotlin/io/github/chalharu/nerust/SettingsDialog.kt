package io.github.chalharu.nerust

import android.os.Build
import androidx.compose.foundation.clickable
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialExpressiveTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.rememberSliderState
import androidx.compose.material3.Surface
import androidx.compose.material3.Slider
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.ChevronRight
import androidx.compose.material.icons.filled.Close
import kotlin.math.roundToLong
import org.json.JSONObject

/** Settings section fallback labels: Rust pushes the label per section.
 * Used only for test-constructed content without a pushed label. */
internal object SettingsSectionFallback {
    const val AUDIO = "Audio"
    const val VIDEO = "Video"
    const val CONTROLS = "Controls"
    const val STORAGE = "Storage"
    const val NOTIFICATIONS = "Notifications"
    const val GENERAL = "General"

    fun labelFor(id: String): String =
        when (id) {
            "audio" -> AUDIO
            "video" -> VIDEO
            "controls" -> CONTROLS
            "storage" -> STORAGE
            "notifications" -> NOTIFICATIONS
            "general" -> GENERAL
            "system.nes" -> "Nintendo Entertainment System"
            "system.gbc" -> "Game Boy Color"
            "system.gba" -> "Game Boy Advance"
            else -> id.substringAfterLast('.').replace('_', ' ').replaceFirstChar(Char::uppercase)
        }
}

/** One pick-list entry: the value rides back to Rust by index, so Kotlin
 * only needs the label plus whether the choice is currently selectable. */
internal data class SettingsChoice(
    val label: String,
    val enabled: Boolean = true,
    val reason: String = "",
)

/** One settings row as projected by Rust (`settings_views::RowWire`). */
internal data class SettingsRow(
    val key: String,
    val label: String,
    val kind: String,
    val choices: List<SettingsChoice>,
    val selected: Int,
    val number: Long,
    val step: Long,
    val min: Long = 0L,
    val max: Long = 0L,
    val unit: String,
    val text: String,
    val target: String = "",
    val visible: Boolean = true,
)

/** One settings section as projected by Rust (`settings_views::SectionWire`). */
internal data class SettingsSection(
    val id: String,
    val label: String,
    val rows: List<SettingsRow>,
    val visible: Boolean = true,
)

internal fun parseSettingsSections(payload: JSONObject): List<SettingsSection> {
    val out = mutableListOf<SettingsSection>()
    val sections = payload.getJSONArray("sections")
    for (i in 0 until sections.length()) {
        val section = sections.getJSONObject(i)
        val rows = mutableListOf<SettingsRow>()
        val fields = section.getJSONArray("rows")
        for (j in 0 until fields.length()) {
            val row = fields.getJSONObject(j)
            val choices = mutableListOf<SettingsChoice>()
            val options = row.optJSONArray("choices")
            if (options != null) {
                for (k in 0 until options.length()) {
                    val option = options.getJSONObject(k)
                    choices +=
                        SettingsChoice(
                            label = option.getString("label"),
                            enabled = option.optBoolean("enabled", true),
                            reason = option.optString("reason", ""),
                        )
                }
            }
            rows +=
                SettingsRow(
                    key = row.getString("key"),
                    label = row.optString("label", ""),
                    kind = row.optString("kind", "info"),
                    choices = choices,
                    selected = row.optInt("selected", 0),
                    number = row.optLong("number", 0L),
                    step = row.optLong("step", 1L).coerceAtLeast(1L),
                    min = row.optLong("min", 0L),
                    max = row.optLong("max", 0L),
                    unit = row.optString("unit", ""),
                    text = row.optString("text", ""),
                    target = row.optString("target", ""),
                    visible = row.optBoolean("visible", true),
                )
        }
        val id = section.getString("id")
        val label = section.optString("label").ifBlank { SettingsSectionFallback.labelFor(id) }
        val visible = section.optBoolean("visible", true)
        out += SettingsSection(id = id, label = label, rows = rows.filter { it.visible }, visible = visible)
    }
    return out
}

internal fun settingsCommandJson(
    key: String,
    index: Int,
    number: Long,
    op: String = "set",
): String =
    JSONObject()
        .put("key", key)
        .put("index", index)
        .put("number", number)
        .put("op", op)
        .toString()

/** Cross-boundary navigation actions the activity needs for system back,
 * discard confirmation, and hardware-key payload delivery. */
internal data class SettingsNavActions(
    val canNavigateUp: Boolean,
    val isDirty: Boolean,
    val isCapturing: Boolean,
    val isConfirming: Boolean,
    val navigateUp: () -> Unit,
    val requestCancel: () -> Unit,
    val dismissConfirm: () -> Unit,
    val cancelCapture: () -> Unit,
    val applyPayload: (String) -> Unit,
)

@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
internal fun NerustExpressiveHost(content: @Composable () -> Unit) {
    val context = LocalContext.current
    val darkTheme = isSystemInDarkTheme()
    val dynamicScheme =
        remember(context, darkTheme) {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                if (darkTheme) dynamicDarkColorScheme(context) else dynamicLightColorScheme(context)
            } else {
                null
            }
        }
    if (dynamicScheme != null) {
        MaterialExpressiveTheme(colorScheme = dynamicScheme) {
            content()
        }
    } else {
        MaterialExpressiveTheme {
            content()
        }
    }
}

@Composable
private fun NerustDialogHost(content: @Composable () -> Unit) {
    Box(
        modifier =
            Modifier
                .fillMaxSize()
                .padding(24.dp),
        contentAlignment = Alignment.Center,
    ) {
        content()
    }
}

@Composable
private fun NerustDialogCard(
    title: String,
    buttons: @Composable RowScope.() -> Unit,
    body: @Composable ColumnScope.() -> Unit,
) {
    Surface(
        modifier = Modifier.widthIn(min = 280.dp, max = 420.dp),
        shape = MaterialTheme.shapes.extraLarge,
        tonalElevation = 6.dp,
    ) {
        Column(modifier = Modifier.padding(24.dp)) {
            Text(text = title, style = MaterialTheme.typography.headlineSmall)
            Spacer(modifier = Modifier.height(16.dp))
            body()
            Spacer(modifier = Modifier.height(24.dp))
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.End,
                content = buttons,
            )
        }
    }
}

@Composable
internal fun NerustErrorDialogCard(
    message: String,
    onDismissRequest: () -> Unit,
) {
    NerustExpressiveHost {
        NerustDialogHost {
            NerustDialogCard(
                title = "Error",
                buttons = {
                    TextButton(onClick = onDismissRequest) { Text("OK") }
                },
            ) {
                SelectionContainer {
                    Text(
                        text = message,
                        modifier =
                            Modifier
                                .heightIn(max = 480.dp)
                                .verticalScroll(rememberScrollState()),
                    )
                }
            }
        }
    }
}

internal fun settingsContentDescription(
    sections: List<SettingsSection>,
    title: String = "Settings",
): String =
    buildList {
        add(title)
        sections
            .filter { it.visible }
            .flatMap { it.rows }
            .filter { it.kind != "header" }
            .forEach { row ->
                add("${row.label}: ${settingsRowDisplayValue(row)}")
            }
    }.joinToString("\n")

internal fun settingsRowDisplayValue(row: SettingsRow): String =
    when (row.kind) {
        "choice" -> row.choices.getOrNull(row.selected)?.label ?: "?"
        "toggle" -> row.choices.getOrNull(row.selected)?.label ?: if (row.selected == 1) "On" else "Off"
        "number" -> "${row.number}${row.unit}"
        else -> row.text
    }

/** Fallback when a settings command fails before Rust answers. */
private const val COMMAND_FAILED = "command failed"

/** DTO-driven settings screen: sections from Rust, one synchronous command
 * per edit, save/dismiss only at the end. */
@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
internal fun NerustSettingsDialogCard(
    initialSections: List<SettingsSection>,
    onDismissRequest: () -> Unit,
    onSave: () -> Unit,
    onCommand: (String) -> String?,
    onNavStateChanged: (SettingsNavActions) -> Unit,
    initialIssues: List<String> = emptyList(),
    initialCapture: String? = null,
) {
    var sections by remember(initialSections) { mutableStateOf(initialSections) }
    var issues by remember(initialIssues) { mutableStateOf(initialIssues) }
    var capturePrompt by remember(initialCapture) { mutableStateOf(initialCapture) }
    var activeSectionId by remember { mutableStateOf<String?>(null) }
    var activeRowKey by remember { mutableStateOf<String?>(null) }
    var commandError by remember { mutableStateOf<String?>(null) }
    var confirmDiscard by remember { mutableStateOf(false) }
    var isDirty by remember { mutableStateOf(false) }

    fun applyPayloadJson(raw: String): Boolean {
        try {
            val doc = JSONObject(raw)
            if (!doc.optBoolean("ok", false)) {
                commandError = doc.optString("error", COMMAND_FAILED)
                return false
            }
            val payload = doc.getJSONObject("payload")
            commandError = null
            sections = parseSettingsSections(payload)
            issues = parseIssues(payload)
            capturePrompt = parseCapture(payload)
            return true
        } catch (e: Exception) {
            commandError = e.message ?: COMMAND_FAILED
            return false
        }
    }

    fun sendCommand(
        key: String,
        index: Int,
        number: Long,
        op: String = "set",
        mutating: Boolean = true,
    ) {
        val raw =
            try {
                onCommand(settingsCommandJson(key, index, number, op))
                    ?: throw IllegalStateException("settings bridge returned null")
            } catch (e: Exception) {
                commandError = e.message ?: COMMAND_FAILED
                return
            }
        if (applyPayloadJson(raw) && mutating) isDirty = true
    }

    val activeSection = sections.find { it.id == activeSectionId }
    val activeRow = activeSection?.rows?.find { it.key == activeRowKey }
    val canNavigateUp = activeSection != null || activeRow != null
    val isCapturing = capturePrompt != null

    fun navigateUp() {
        if (activeRow != null) {
            activeRowKey = null
        } else {
            activeSectionId = null
        }
    }

    fun requestCancel() {
        if (isDirty) {
            confirmDiscard = true
        } else {
            onDismissRequest()
        }
    }

    // Publish exactly when navigation-relevant state changes; calling the
    // callback directly during composition would re-trigger on every recompose.
    LaunchedEffect(canNavigateUp, isDirty, isCapturing, confirmDiscard) {
        onNavStateChanged(
            SettingsNavActions(
                canNavigateUp = canNavigateUp,
                isDirty = isDirty,
                isCapturing = isCapturing,
                isConfirming = confirmDiscard,
                navigateUp = ::navigateUp,
                requestCancel = ::requestCancel,
                dismissConfirm = { confirmDiscard = false },
                cancelCapture = { sendCommand("", 0, 0L, "cancel_capture", mutating = false) },
                applyPayload = { raw ->
                    if (applyPayloadJson(raw)) isDirty = true
                },
            ),
        )
    }

    val screenTitle = activeRow?.label ?: activeSection?.label ?: stringResource(R.string.settings_title)
    val screenSubtitle =
        when {
            activeRow != null -> activeSection?.label
            activeSection != null -> stringResource(R.string.settings_title)
            else -> null
        }

    NerustExpressiveHost {
        Scaffold(
            topBar = {
                TopAppBar(
                    title = {
                        Column {
                            Text(text = screenTitle)
                            screenSubtitle?.let {
                                Text(
                                    text = it,
                                    style = MaterialTheme.typography.labelMedium,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                            }
                        }
                    },
                    navigationIcon = {
                        if (canNavigateUp) {
                            IconButton(onClick = ::navigateUp) {
                                Icon(
                                    imageVector = Icons.AutoMirrored.Filled.ArrowBack,
                                    contentDescription = stringResource(R.string.settings_back),
                                )
                            }
                        } else {
                            IconButton(onClick = ::requestCancel) {
                                Icon(
                                    imageVector = Icons.Filled.Close,
                                    contentDescription = stringResource(R.string.settings_close),
                                )
                            }
                        }
                    },
                    actions = {
                        FilledTonalButton(onClick = onSave, enabled = isDirty) {
                            Text(stringResource(R.string.settings_save))
                        }
                    },
                    colors =
                        TopAppBarDefaults.topAppBarColors(
                            containerColor = MaterialTheme.colorScheme.surfaceContainer,
                        ),
                )
            },
            containerColor = MaterialTheme.colorScheme.surfaceContainerLow,
        ) { padding ->
            SettingsScreenContent(
                modifier =
                    Modifier
                        .fillMaxSize()
                        .padding(padding)
                        .padding(horizontal = 16.dp),
                sections = sections,
                activeSection = activeSection,
                activeRow = activeRow,
                issues = issues,
                capturePrompt = capturePrompt,
                commandError = commandError,
                onCancelCapture = { sendCommand("", 0, 0L, "cancel_capture", mutating = false) },
                onSelectChoice = { row, choiceIndex ->
                    sendCommand(row.key, choiceIndex, 0L)
                    activeRowKey = null
                },
                onToggle = { row, checked ->
                    sendCommand(row.key, if (checked) 1 else 0, 0L)
                },
                onCommitNumber = { row, value ->
                    sendCommand(row.key, 0, value)
                },
                onRowClick = { row ->
                    when (row.kind) {
                        "group" -> {
                            if (sections.any { it.id == row.target }) {
                                activeRowKey = null
                                activeSectionId = row.target
                            }
                        }
                        "capture" -> {
                            sendCommand(row.key, 0, 0L, "start_capture", mutating = false)
                        }
                        "choice" -> activeRowKey = row.key
                        // toggle/number/info act inline or are static;
                        // never push a detail level.
                        else -> Unit
                    }
                },
                onClearCapture = { row ->
                    sendCommand(row.key, 0, 0L, "clear_binding")
                },
                onSectionClick = { activeSectionId = it },
            )
        }
        if (confirmDiscard) {
            androidx.compose.material3.AlertDialog(
                onDismissRequest = { confirmDiscard = false },
                title = { Text(stringResource(R.string.settings_discard_title)) },
                text = { Text(stringResource(R.string.settings_discard_message)) },
                confirmButton = {
                    TextButton(
                        onClick = {
                            confirmDiscard = false
                            onDismissRequest()
                        },
                    ) { Text(stringResource(R.string.settings_discard_confirm)) }
                },
                dismissButton = {
                    TextButton(onClick = { confirmDiscard = false }) {
                        Text(stringResource(R.string.settings_keep_editing))
                    }
                },
            )
        }
    }
}

internal fun parseIssues(payload: JSONObject): List<String> {
    val out = mutableListOf<String>()
    val issues = payload.optJSONArray("issues") ?: return out
    for (i in 0 until issues.length()) {
        out += issues.optString(i)
    }
    return out
}

internal fun parseCapture(payload: JSONObject): String? =
    payload.optJSONObject("capture")?.optString("prompt")?.ifBlank { null }

@Composable
private fun SettingsScreenContent(
    modifier: Modifier,
    sections: List<SettingsSection>,
    activeSection: SettingsSection?,
    activeRow: SettingsRow?,
    issues: List<String>,
    capturePrompt: String?,
    commandError: String?,
    onCancelCapture: () -> Unit,
    onSelectChoice: (SettingsRow, Int) -> Unit,
    onToggle: (SettingsRow, Boolean) -> Unit,
    onCommitNumber: (SettingsRow, Long) -> Unit,
    onRowClick: (SettingsRow) -> Unit,
    onClearCapture: (SettingsRow) -> Unit,
    onSectionClick: (String) -> Unit,
) {
    Column(modifier = modifier) {
        capturePrompt?.let { prompt ->
            Card(
                colors =
                    CardDefaults.cardColors(
                        containerColor = MaterialTheme.colorScheme.primaryContainer,
                    ),
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .padding(vertical = 8.dp),
            ) {
                Column(modifier = Modifier.padding(16.dp)) {
                    Text(
                        text = prompt,
                        style = MaterialTheme.typography.bodyLarge,
                        color = MaterialTheme.colorScheme.onPrimaryContainer,
                    )
                    Spacer(modifier = Modifier.height(8.dp))
                    TextButton(onClick = onCancelCapture) {
                        Text(stringResource(R.string.settings_cancel))
                    }
                }
            }
        }
        if (issues.isNotEmpty()) {
            Card(
                colors =
                    CardDefaults.cardColors(
                        containerColor = MaterialTheme.colorScheme.errorContainer,
                    ),
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .padding(vertical = 8.dp),
            ) {
                Column(modifier = Modifier.padding(16.dp)) {
                    Text(
                        text = stringResource(R.string.settings_needs_attention),
                        style = MaterialTheme.typography.titleSmall,
                        color = MaterialTheme.colorScheme.onErrorContainer,
                    )
                    Spacer(modifier = Modifier.height(4.dp))
                    issues.forEach { issue ->
                        Text(
                            text = "\u2022 $issue",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onErrorContainer,
                        )
                    }
                }
            }
        }
        commandError?.let {
            Column(modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp)) {
                Text(
                    text = stringResource(R.string.settings_update_failed),
                    style = MaterialTheme.typography.titleSmall,
                    color = MaterialTheme.colorScheme.error,
                )
                Text(
                    text = it,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.error,
                )
            }
        }
        when {
            activeRow != null && activeRow.kind == "choice" -> {
                SettingsChoiceList(
                    row = activeRow,
                    onSelect = { choiceIndex -> onSelectChoice(activeRow, choiceIndex) },
                )
            }

            activeSection != null -> {
                SettingsSectionList(
                    section = activeSection,
                    onToggle = onToggle,
                    onCommitNumber = onCommitNumber,
                    onRowClick = onRowClick,
                    onClearCapture = onClearCapture,
                )
            }

            else -> {
                SettingsCategoryList(
                    sections = sections.filter { it.visible },
                    onSectionClick = onSectionClick,
                )
            }
        }
    }
}

@Composable
private fun SettingsCategoryList(
    sections: List<SettingsSection>,
    onSectionClick: (String) -> Unit,
) {
    LazyColumn(
        modifier = Modifier.fillMaxSize(),
        verticalArrangement = Arrangement.spacedBy(12.dp),
        contentPadding = PaddingValues(vertical = 12.dp),
    ) {
        itemsIndexed(sections, key = { _, section -> section.id }) { _, section ->
            // Summaries show only inline-editable rows; group/capture/info
            // rows would leak a bare chevron or a binding label here.
            val summary =
                section.rows
                    .filter { it.kind == "choice" || it.kind == "toggle" || it.kind == "number" }
                    .take(2)
                    .joinToString(" · ") { row ->
                        "${row.label}: ${settingsRowDisplayValue(row)}"
                    }
            Card(
                onClick = { onSectionClick(section.id) },
                shape = RoundedCornerShape(28.dp),
                colors =
                    CardDefaults.cardColors(
                        containerColor = MaterialTheme.colorScheme.surfaceContainerHigh,
                    ),
                modifier = Modifier.fillMaxWidth(),
            ) {
                Column(modifier = Modifier.padding(20.dp)) {
                    Text(
                        text = section.label,
                        style = MaterialTheme.typography.titleLarge,
                    )
                    Text(
                        text =
                            pluralStringResource(
                                R.plurals.settings_count,
                                section.rows.size,
                                section.rows.size,
                            ),
                        style = MaterialTheme.typography.labelLarge,
                        color = MaterialTheme.colorScheme.primary,
                    )
                    if (summary.isNotBlank()) {
                        Spacer(modifier = Modifier.height(4.dp))
                        Text(
                            text = summary,
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun SettingsSectionList(
    section: SettingsSection,
    onToggle: (SettingsRow, Boolean) -> Unit,
    onCommitNumber: (SettingsRow, Long) -> Unit,
    onRowClick: (SettingsRow) -> Unit,
    onClearCapture: (SettingsRow) -> Unit,
) {
    // Split rows into groups at header rows; each group renders as one card.
    val groups = remember(section) { splitSectionGroups(section.rows) }
    LazyColumn(
        modifier = Modifier.fillMaxSize(),
        verticalArrangement = Arrangement.spacedBy(12.dp),
        contentPadding = PaddingValues(vertical = 12.dp),
    ) {
        groups.forEach { group ->
            if (group.title.isNotBlank()) {
                item(key = "${section.id}#${group.title}") {
                    Text(
                        text = group.title,
                        style = MaterialTheme.typography.titleSmall,
                        color = MaterialTheme.colorScheme.primary,
                        modifier = Modifier.padding(start = 8.dp, top = 8.dp, bottom = 4.dp),
                    )
                }
            }
            item(key = "${section.id}#${group.title}#card") {
                Card(
                    shape = RoundedCornerShape(28.dp),
                    colors =
                        CardDefaults.cardColors(
                            containerColor = MaterialTheme.colorScheme.surfaceContainer,
                        ),
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    Column {
                        group.rows.forEachIndexed { index, row ->
                            when (row.kind) {
                                "toggle" -> {
                                    SettingsToggleRow(
                                        row = row,
                                        onToggle = { onToggle(row, it) },
                                    )
                                }
                                "number" -> {
                                    SettingsSliderRow(
                                        row = row,
                                        onCommit = { onCommitNumber(row, it) },
                                    )
                                }
                                "choice" -> {
                                    SettingsNavigationRow(
                                        title = row.label,
                                        value = settingsRowDisplayValue(row),
                                        semanticsLabel = "${row.key}: ${row.label}: ${settingsRowDisplayValue(row)}",
                                        onClick = { onRowClick(row) },
                                    )
                                }
                                "group" -> {
                                    SettingsNavigationRow(
                                        title = row.label,
                                        value = "",
                                        semanticsLabel = "${row.key}: ${row.label}",
                                        showChevron = true,
                                        onClick = { onRowClick(row) },
                                    )
                                }
                                "capture" -> {
                                    SettingsCaptureRow(
                                        row = row,
                                        onStart = { onRowClick(row) },
                                        onClear = { onClearCapture(row) },
                                    )
                                }
                                else -> {
                                    SettingsInfoRow(
                                        title = row.label,
                                        value = settingsRowDisplayValue(row),
                                        semanticsLabel = "${row.key}: ${row.label}",
                                    )
                                }
                            }
                            if (index < group.rows.lastIndex) HorizontalDivider()
                        }
                    }
                }
            }
        }
    }
}

private data class SettingsRowGroup(
    val title: String,
    val rows: List<SettingsRow>,
)

private fun splitSectionGroups(rows: List<SettingsRow>): List<SettingsRowGroup> {
    val groups = mutableListOf<SettingsRowGroup>()
    var title = ""
    var current = mutableListOf<SettingsRow>()
    for (row in rows) {
        if (row.kind == "header") {
            if (current.isNotEmpty()) {
                groups += SettingsRowGroup(title, current)
                current = mutableListOf()
            }
            title = row.label
        } else {
            current += row
        }
    }
    if (current.isNotEmpty() || groups.isEmpty()) {
        groups += SettingsRowGroup(title, current)
    }
    return groups
}

@Composable
private fun SettingsToggleRow(
    row: SettingsRow,
    onToggle: (Boolean) -> Unit,
) {
    val checked = row.selected == 1
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .toggleable(
                    value = checked,
                    role = Role.Switch,
                    onValueChange = onToggle,
                )
                .padding(horizontal = 24.dp, vertical = 12.dp)
                .semantics { contentDescription = "${row.key}: ${row.label}" },
        horizontalArrangement = Arrangement.SpaceBetween,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            text = row.label,
            modifier = Modifier.weight(1f),
            style = MaterialTheme.typography.titleMedium,
        )
        // Row owns the toggle; the switch is a static indicator so a tap
        // never fires twice and TalkBack announces a single switch.
        Switch(
            checked = checked,
            onCheckedChange = null,
        )
    }
}

@Composable
private fun SettingsSliderRow(
    row: SettingsRow,
    onCommit: (Long) -> Unit,
) {
    val span = (row.max - row.min).coerceAtLeast(0L)
    val step = row.step.coerceAtLeast(1L)
    fun snap(value: Long): Long {
        val steps = ((value - row.min).toDouble() / step).roundToLong()
        return (row.min + steps * step).coerceIn(row.min, row.min + span)
    }
    val sliderState =
        rememberSliderState(
            value = snap(row.number).toFloat(),
            trackRange = row.min.toFloat()..(row.min + span).toFloat(),
        )
    LaunchedEffect(row.key, row.number) {
        sliderState.value = snap(row.number).toFloat()
    }
    // No snap during the drag: the thumb tracks the finger and only the
    // label previews the snapped step; the commit snaps on release.
    Column(
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(horizontal = 24.dp, vertical = 12.dp)
                .semantics { contentDescription = "${row.key}: ${row.label}" },
    ) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                text = row.label,
                style = MaterialTheme.typography.titleMedium,
                modifier = Modifier.weight(1f),
            )
            Text(
                text = "${snap(sliderState.value.roundToLong())}${row.unit}",
                style = MaterialTheme.typography.bodyMedium,
            )
        }
        Spacer(modifier = Modifier.height(8.dp))
        Slider(
            state = sliderState,
            onValueChange = { v -> sliderState.value = v },
            onValueChangeFinished = { onCommit(snap(sliderState.value.roundToLong())) },
        )
    }
}

@Composable
private fun SettingsCaptureRow(
    row: SettingsRow,
    onStart: () -> Unit,
    onClear: () -> Unit,
) {
    // Tap targets are siblings, not nested: the label column starts
    // capture, the trailing button clears. Nested clickables would let a
    // Clear tap also start a capture.
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(horizontal = 24.dp, vertical = 12.dp)
                .semantics { contentDescription = "${row.key}: ${row.label}" },
        horizontalArrangement = Arrangement.SpaceBetween,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(
            modifier =
                Modifier
                    .weight(1f)
                    .clickable(onClick = onStart)
                    .padding(vertical = 4.dp),
        ) {
            Text(text = row.label, style = MaterialTheme.typography.titleMedium)
            Text(
                text = row.text.ifBlank { stringResource(R.string.settings_tap_to_assign) },
                style = MaterialTheme.typography.bodyMedium,
            )
        }
        TextButton(onClick = onClear) { Text(stringResource(R.string.settings_clear)) }
    }
}

@Composable
private fun SettingsChoiceList(
    row: SettingsRow,
    onSelect: (Int) -> Unit,
) {
    LazyColumn(
        modifier = Modifier.fillMaxSize(),
        verticalArrangement = Arrangement.spacedBy(12.dp),
        contentPadding = PaddingValues(vertical = 12.dp),
    ) {
        item(key = "${row.key}#card") {
            Card(
                shape = RoundedCornerShape(28.dp),
                colors =
                    CardDefaults.cardColors(
                        containerColor = MaterialTheme.colorScheme.surfaceContainer,
                    ),
                modifier = Modifier.fillMaxWidth(),
            ) {
                Column {
                    row.choices.forEachIndexed { choiceIndex, choice ->
                        val reason =
                            choice.reason.ifBlank { null }?.let { owner ->
                                if (owner.isBlank()) {
                                    stringResource(R.string.settings_used_elsewhere)
                                } else {
                                    // Rust passes the owner slot label; Kotlin owns the sentence.
                                    LocalContext.current.getString(R.string.settings_used_in, owner)
                                }
                            }
                        Row(
                            modifier =
                                Modifier
                                    .fillMaxWidth()
                                    .clickable(
                                        enabled = choice.enabled,
                                        onClick = { onSelect(choiceIndex) },
                                    )
                                    .padding(vertical = 12.dp, horizontal = 20.dp)
                                    .semantics { contentDescription = choice.label },
                            horizontalArrangement = Arrangement.SpaceBetween,
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Column(modifier = Modifier.weight(1f)) {
                                Text(
                                    text = choice.label,
                                    color =
                                        if (choice.enabled) {
                                            MaterialTheme.colorScheme.onSurface
                                        } else {
                                            MaterialTheme.colorScheme.onSurfaceVariant
                                        },
                                )
                                if (!choice.enabled) {
                                    Text(
                                        text =
                                            reason
                                                ?: stringResource(R.string.settings_used_elsewhere),
                                        style = MaterialTheme.typography.bodySmall,
                                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                                    )
                                }
                            }
                            RadioButton(
                                selected = row.selected == choiceIndex,
                                onClick = null,
                                enabled = choice.enabled,
                            )
                        }
                        if (choiceIndex < row.choices.lastIndex) HorizontalDivider()
                    }
                }
            }
        }
    }
}

@Composable
private fun SettingsNavigationRow(
    title: String,
    value: String,
    semanticsLabel: String,
    supportingText: String? = null,
    showChevron: Boolean = false,
    onClick: () -> Unit,
) {
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .clickable(onClick = onClick)
                .padding(horizontal = 24.dp, vertical = 18.dp)
                .semantics { contentDescription = semanticsLabel },
        horizontalArrangement = Arrangement.SpaceBetween,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(modifier = Modifier.weight(1f)) {
            Text(text = title, style = MaterialTheme.typography.titleMedium)
            supportingText?.takeIf(String::isNotBlank)?.let {
                Text(text = it, style = MaterialTheme.typography.bodySmall)
            }
        }
        if (value.isNotBlank()) {
            Text(text = value, style = MaterialTheme.typography.bodyMedium)
        }
        if (showChevron) {
            Icon(
                imageVector = Icons.Filled.ChevronRight,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

@Composable
private fun SettingsInfoRow(
    title: String,
    value: String,
    semanticsLabel: String,
) {
    // Static readout: deliberately not clickable so it never pushes a
    // detail level that renders nothing.
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(horizontal = 24.dp, vertical = 18.dp)
                .semantics { contentDescription = semanticsLabel },
        horizontalArrangement = Arrangement.SpaceBetween,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            text = title,
            style = MaterialTheme.typography.titleMedium,
            modifier = Modifier.weight(1f),
        )
        Text(text = value, style = MaterialTheme.typography.bodyMedium)
    }
}

internal fun buildSettingsSections(sections: List<SettingsSection>): List<SettingsSection> = sections

internal fun settingsCountLabel(count: Int): String = "$count ${if (count == 1) "setting" else "settings"}"

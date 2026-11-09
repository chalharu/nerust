package io.github.chalharu.nerust

import android.os.Build
import androidx.compose.foundation.clickable
import androidx.compose.foundation.isSystemInDarkTheme
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
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
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
import androidx.compose.material3.FilledTonalIconButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialExpressiveTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
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
 * only needs the label. */
internal data class SettingsChoice(
    val label: String,
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
    val unit: String,
    val text: String,
    val visible: Boolean = true,
)

/** One settings section as projected by Rust (`settings_views::SectionWire`). */
internal data class SettingsSection(
    val id: String,
    val label: String,
    val rows: List<SettingsRow>,
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
                    choices += SettingsChoice(label = options.getJSONObject(k).getString("label"))
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
                    unit = row.optString("unit", ""),
                    text = row.optString("text", ""),
                    visible = row.optBoolean("visible", true),
                )
        }
        val id = section.getString("id")
        val label = section.optString("label").ifBlank { SettingsSectionFallback.labelFor(id) }
        out += SettingsSection(id = id, label = label, rows = rows.filter { it.visible })
    }
    return out
}

internal fun settingsCommandJson(
    key: String,
    index: Int,
    number: Long,
): String =
    JSONObject()
        .put("key", key)
        .put("index", index)
        .put("number", number)
        .toString()

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

internal fun settingsContentDescription(sections: List<SettingsSection>): String =
    buildList {
        add("Settings")
        sections.flatMap { it.rows }.filter { it.kind != "header" }.forEach { row ->
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
) {
    var sections by remember(initialSections) { mutableStateOf(initialSections) }
    var activeSectionId by remember { mutableStateOf<String?>(null) }
    var activeRowKey by remember { mutableStateOf<String?>(null) }
    var commandError by remember { mutableStateOf<String?>(null) }

    fun sendCommand(
        key: String,
        index: Int,
        number: Long,
    ) {
        val raw =
            try {
                onCommand(settingsCommandJson(key, index, number))
                    ?: throw IllegalStateException("settings bridge returned null")
            } catch (e: Exception) {
                commandError = e.message ?: COMMAND_FAILED
                return
            }
        try {
            val doc = JSONObject(raw)
            if (!doc.optBoolean("ok", false)) {
                commandError = doc.optString("error", COMMAND_FAILED)
                return
            }
            commandError = null
            sections = parseSettingsSections(doc.getJSONObject("payload"))
        } catch (e: Exception) {
            commandError = e.message ?: COMMAND_FAILED
        }
    }

    val activeSection = sections.find { it.id == activeSectionId }
    val activeRow = activeSection?.rows?.find { it.key == activeRowKey }
    val title =
        activeRow?.label
            ?: activeSection?.label
            ?: "Settings"
    val canNavigateBack = activeSection != null || activeRow != null

    NerustExpressiveHost {
        Scaffold(
            topBar = {
                TopAppBar(
                    title = { Text(text = title) },
                    navigationIcon = {
                        if (canNavigateBack) {
                            TextButton(onClick = {
                                if (activeRow != null) {
                                    activeRowKey = null
                                } else {
                                    activeSectionId = null
                                }
                            }) { Text("Back") }
                        } else {
                            TextButton(onClick = onDismissRequest) { Text("Cancel") }
                        }
                    },
                    actions = {
                        FilledTonalButton(onClick = onSave) { Text("Save") }
                    },
                    colors =
                        TopAppBarDefaults.topAppBarColors(
                            containerColor = MaterialTheme.colorScheme.surfaceContainer,
                        ),
                )
            },
            containerColor = MaterialTheme.colorScheme.surfaceContainerLow,
        ) { padding ->
            Column(
                modifier =
                    Modifier
                        .fillMaxSize()
                        .padding(padding)
                        .padding(horizontal = 16.dp),
            ) {
                commandError?.let {
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
                        Text(
                            text = it,
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onErrorContainer,
                            modifier = Modifier.padding(16.dp),
                        )
                    }
                }
                when {
                    activeRow != null && activeRow.kind == "choice" -> {
                        SettingsChoiceList(
                            row = activeRow,
                            onSelect = { choiceIndex ->
                                sendCommand(activeRow.key, choiceIndex, 0L)
                                activeRowKey = null
                            },
                        )
                    }

                    activeSection != null -> {
                        SettingsSectionList(
                            section = activeSection,
                            onToggle = { row, checked ->
                                sendCommand(row.key, if (checked) 1 else 0, 0L)
                            },
                            onStep = { row, delta ->
                                sendCommand(row.key, 0, row.number + delta * row.step)
                            },
                            onRowClick = { activeRowKey = it },
                        )
                    }

                    else -> {
                        SettingsCategoryList(
                            sections = sections,
                            onSectionClick = { activeSectionId = it },
                        )
                    }
                }
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
        contentPadding = androidx.compose.foundation.layout.PaddingValues(vertical = 12.dp),
    ) {
        itemsIndexed(sections, key = { _, section -> section.id }) { _, section ->
            val summary =
                section.rows.filter { it.kind != "header" }.take(2).joinToString(" · ") { row ->
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
                        text = settingsCountLabel(section.rows.size),
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
    onStep: (SettingsRow, Long) -> Unit,
    onRowClick: (String) -> Unit,
) {
    // Split rows into groups at header rows; each group renders as one card.
    val groups = remember(section) { splitSectionGroups(section.rows) }
    LazyColumn(
        modifier = Modifier.fillMaxSize(),
        verticalArrangement = Arrangement.spacedBy(12.dp),
        contentPadding = androidx.compose.foundation.layout.PaddingValues(vertical = 12.dp),
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
                            containerColor = MaterialTheme.colorScheme.surfaceContainerHigh,
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
                                    SettingsStepperRow(
                                        row = row,
                                        onStep = { onStep(row, it) },
                                    )
                                }
                                "choice" -> {
                                    SettingsNavigationRow(
                                        title = row.label,
                                        value = settingsRowDisplayValue(row),
                                        semanticsLabel = "${row.key}: ${row.label}: ${settingsRowDisplayValue(row)}",
                                        onClick = { onRowClick(row.key) },
                                    )
                                }
                                else -> {
                                    SettingsNavigationRow(
                                        title = row.label,
                                        value = settingsRowDisplayValue(row),
                                        semanticsLabel = "${row.key}: ${row.label}",
                                        onClick = {},
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
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .clickable(onClick = { onToggle(row.selected != 1) })
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
        Switch(
            checked = row.selected == 1,
            onCheckedChange = onToggle,
        )
    }
}

@Composable
private fun SettingsStepperRow(
    row: SettingsRow,
    onStep: (Long) -> Unit,
) {
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(horizontal = 24.dp, vertical = 12.dp)
                .semantics { contentDescription = "${row.key}: ${row.label}" },
        horizontalArrangement = Arrangement.SpaceBetween,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(modifier = Modifier.weight(1f)) {
            Text(text = row.label, style = MaterialTheme.typography.titleMedium)
            Text(
                text = "${row.number}${row.unit}",
                style = MaterialTheme.typography.bodyMedium,
            )
        }
        Row(verticalAlignment = Alignment.CenterVertically) {
            FilledTonalIconButton(onClick = { onStep(-1) }) { Text("−") }
            Spacer(modifier = Modifier.width(8.dp))
            FilledTonalIconButton(onClick = { onStep(1) }) { Text("+") }
        }
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
        contentPadding = androidx.compose.foundation.layout.PaddingValues(vertical = 12.dp),
    ) {
        item(key = "${row.key}#card") {
            Card(
                shape = RoundedCornerShape(28.dp),
                colors =
                    CardDefaults.cardColors(
                        containerColor = MaterialTheme.colorScheme.surfaceContainerHigh,
                    ),
                modifier = Modifier.fillMaxWidth(),
            ) {
                Column {
                    row.choices.forEachIndexed { choiceIndex, choice ->
                        Row(
                            modifier =
                                Modifier
                                    .fillMaxWidth()
                                    .clickable(onClick = { onSelect(choiceIndex) })
                                    .padding(vertical = 12.dp, horizontal = 20.dp)
                                    .semantics { contentDescription = choice.label },
                            horizontalArrangement = Arrangement.SpaceBetween,
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Text(text = choice.label, modifier = Modifier.weight(1f))
                            RadioButton(selected = row.selected == choiceIndex, onClick = null)
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
        Text(text = value, style = MaterialTheme.typography.bodyMedium)
    }
}

internal fun buildSettingsSections(sections: List<SettingsSection>): List<SettingsSection> = sections

internal fun settingsCountLabel(count: Int): String = "$count ${if (count == 1) "setting" else "settings"}"

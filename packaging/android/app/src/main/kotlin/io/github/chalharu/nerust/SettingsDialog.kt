package io.github.chalharu.nerust

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Slider
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import kotlin.math.max
import kotlin.math.roundToInt

/** Settings dialog UI: cards, section builders, visibility filtering, and selection state. */
internal data class AndroidSetting(
    val key: String,
    val section: String,
    val sectionLabel: String,
    val label: String,
    val kind: String,
    val choices: List<String>,
    val rangeMin: Int,
    val rangeMax: Int,
    val unit: String,
    val visible: Boolean = true,
)

internal data class AndroidSettingsSection(
    val id: String,
    val label: String,
    val settingIndices: List<Int>,
)

/** One dialog field as pushed by Rust: choices arrive as a JSON array,
 * ranged ints as min/max/unit with an offset value. No string escaping. */
internal data class SettingsField(
    val key: String,
    val section: String,
    val sectionLabel: String,
    val label: String,
    val kind: String,
    val choices: List<String>,
    val value: Int,
    val visible: Boolean,
    val rangeMin: Int = 0,
    val rangeMax: Int = 0,
    val unit: String = "",
)

/** Display value for a row: ranged fields render `min+index` with the unit. */
internal fun settingDisplayValue(setting: AndroidSetting, selection: Int): String =
    if (setting.kind == "range") {
        "${setting.rangeMin + selection}${setting.unit}"
    } else {
        setting.choices.getOrElse(selection) { "?" }
    }

/** Keyed dialog content assembled by MainActivity from the Rust payload. */
internal data class SettingsDialogContent(
    val fields: List<SettingsField>,
)

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
private fun NerustFullScreenDialogHost(content: @Composable ColumnScope.() -> Unit) {
    Surface(modifier = Modifier.fillMaxSize()) {
        Column(
            modifier =
                Modifier
                    .fillMaxSize()
                    .padding(WindowInsets.safeDrawing.asPaddingValues())
                    .padding(horizontal = 24.dp, vertical = 16.dp),
            content = content,
        )
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

internal fun settingsContentDescription(
    settings: List<AndroidSetting>,
    selections: List<Int>,
): String =
    buildList {
        add("Settings")
        settings.forEachIndexed { index, setting ->
            add("${setting.label}: ${settingDisplayValue(setting, selections.getOrElse(index) { 0 })}")
        }
    }.joinToString("\n")

@Composable
internal fun NerustSettingsDialogCard(
    settings: List<AndroidSetting>,
    initialSelections: List<Int>,
    onDismissRequest: () -> Unit,
    onSave: (List<Int>) -> Unit,
) {
    val selections = rememberSettingsSelections(settings, initialSelections)
    val sections = remember(settings) { buildSettingsSections(settings) }
    var activeSectionId by remember { mutableStateOf<String?>(null) }
    var activeSettingIndex by remember { mutableStateOf<Int?>(null) }

    Surface(modifier = Modifier.fillMaxSize()) {
        Column(
            modifier =
                Modifier
                    .fillMaxSize()
                    .padding(WindowInsets.safeDrawing.asPaddingValues()),
        ) {
            val activeSection = sections.find { it.id == activeSectionId }
            val visibleSettingCount =
                activeSection
                    ?.let { visibleSettingIndices(settings, it).size }
            SettingsNavigationHeader(
                title =
                    activeSettingIndex?.let { settings[it].label }
                        ?: activeSection?.label
                        ?: "Settings",
                subtitle =
                    when {
                        activeSettingIndex != null -> "Choose a value"
                        activeSection != null -> settingsCountLabel(visibleSettingCount ?: 0)
                        else -> "Choose a category"
                    },
                canNavigateBack = activeSection != null || activeSettingIndex != null,
                onNavigateBack = {
                    if (activeSettingIndex != null) {
                        activeSettingIndex = null
                    } else {
                        activeSectionId = null
                    }
                },
                onDismissRequest = onDismissRequest,
                onSave = { onSave(selections.toList()) },
            )
            HorizontalDivider()

            when {
                activeSettingIndex != null -> {
                    val settingIndex = requireNotNull(activeSettingIndex)
                    SettingsChoiceList(
                        setting = settings[settingIndex],
                        selectedIndex = selections[settingIndex],
                        onSelect = { choiceIndex ->
                            selections[settingIndex] = choiceIndex
                            if (settings[settingIndex].kind != "range") {
                                activeSettingIndex = null
                            }
                        },
                    )
                }

                activeSection != null -> {
                    val visibleSection =
                        activeSection.copy(
                            settingIndices =
                                visibleSettingIndices(settings, activeSection),
                        )
                    SettingsSectionList(
                        section = visibleSection,
                        settings = settings,
                        selections = selections,
                        onSettingClick = { activeSettingIndex = it },
                    )
                }

                else -> {
                    SettingsCategoryList(
                        sections = sections,
                        settings = settings,
                        selections = selections,
                        onSectionClick = { activeSectionId = it },
                    )
                }
            }
        }
    }
}

@Composable
private fun SettingsNavigationHeader(
    title: String,
    subtitle: String,
    canNavigateBack: Boolean,
    onNavigateBack: () -> Unit,
    onDismissRequest: () -> Unit,
    onSave: () -> Unit,
) {
    Column(modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp)) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            if (canNavigateBack) {
                TextButton(onClick = onNavigateBack) { Text("Back") }
            } else {
                TextButton(onClick = onDismissRequest) { Text("Cancel") }
            }
            TextButton(onClick = onSave) { Text("Save") }
        }
        Text(text = title, style = MaterialTheme.typography.headlineSmall)
        Text(text = subtitle, style = MaterialTheme.typography.bodyMedium)
    }
}

@Composable
private fun SettingsCategoryList(
    sections: List<AndroidSettingsSection>,
    settings: List<AndroidSetting>,
    selections: List<Int>,
    onSectionClick: (String) -> Unit,
) {
    LazyColumn(modifier = Modifier.fillMaxSize()) {
        itemsIndexed(sections) { index, section ->
            val visibleIndices = visibleSettingIndices(settings, section)
            val summary =
                visibleIndices.take(2).joinToString(" · ") { settingIndex ->
                    val setting = settings[settingIndex]
                    "${setting.label}: ${settingDisplayValue(setting, selections[settingIndex])}"
                }
            SettingsNavigationRow(
                title = section.label,
                value = settingsCountLabel(visibleIndices.size),
                supportingText = summary,
                semanticsLabel = "${section.label}: ${settingsCountLabel(visibleIndices.size)}",
                onClick = { onSectionClick(section.id) },
            )
            if (index < sections.lastIndex) HorizontalDivider()
        }
    }
}

@Composable
private fun SettingsSectionList(
    section: AndroidSettingsSection,
    settings: List<AndroidSetting>,
    selections: List<Int>,
    onSettingClick: (Int) -> Unit,
) {
    LazyColumn(modifier = Modifier.fillMaxSize()) {
        itemsIndexed(section.settingIndices) { index, settingIndex ->
            val setting = settings[settingIndex]
            val value = settingDisplayValue(setting, selections[settingIndex])
            SettingsNavigationRow(
                title = setting.label,
                value = value,
                semanticsLabel = "${setting.key}: ${setting.label}: $value",
                onClick = { onSettingClick(settingIndex) },
            )
            if (index < section.settingIndices.lastIndex) HorizontalDivider()
        }
    }
}

@Composable
private fun SettingsChoiceList(
    setting: AndroidSetting,
    selectedIndex: Int,
    onSelect: (Int) -> Unit,
) {
    if (setting.kind == "range") {
        SettingsSliderRow(setting, selectedIndex, onSelect)
        return
    }
    LazyColumn(modifier = Modifier.fillMaxSize()) {
        itemsIndexed(setting.choices) { choiceIndex, choiceLabel ->
            DialogChoiceButton(
                label = choiceLabel,
                selected = selectedIndex == choiceIndex,
            ) {
                onSelect(choiceIndex)
            }
            if (choiceIndex < setting.choices.lastIndex) HorizontalDivider()
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

internal fun buildSettingsSections(settings: List<AndroidSetting>): List<AndroidSettingsSection> =
    settings
        .withIndex()
        .groupBy({ it.value.section }, { it.index })
        .map { (id, indices) ->
            val label =
                indices.firstNotNullOfOrNull { settings[it].sectionLabel.ifBlank { null } }
                    ?: SettingsSectionFallback.labelFor(id)
            AndroidSettingsSection(id, label, indices)
        }

internal fun settingsCountLabel(count: Int): String = "$count ${if (count == 1) "setting" else "settings"}"

/** Native ranged row: a Slider over `min..max` with a value label.
 * Kotlin expands nothing; the index rides back as `value - min`. */
@Composable
private fun SettingsSliderRow(
    setting: AndroidSetting,
    selectedIndex: Int,
    onSelect: (Int) -> Unit,
) {
    val span = (setting.rangeMax - setting.rangeMin).coerceAtLeast(0)
    val clamped = selectedIndex.coerceIn(0, span)
    Column(modifier = Modifier.fillMaxSize().padding(horizontal = 24.dp, vertical = 18.dp)) {
        Text(
            text = "${setting.rangeMin + clamped}${setting.unit}",
            style = MaterialTheme.typography.titleLarge,
        )
        Spacer(modifier = Modifier.height(16.dp))
        Slider(
            value = clamped.toFloat(),
            onValueChange = { onSelect(it.roundToInt().coerceIn(0, span)) },
            valueRange = 0f..span.toFloat(),
            steps = (span - 1).coerceAtLeast(0),
        )
    }
}

// Visibility is resolved in Rust per field; Kotlin only filters on the flag.
internal fun visibleSettingIndices(
    settings: List<AndroidSetting>,
    section: AndroidSettingsSection,
): List<Int> = section.settingIndices.filter { settings[it].visible }

@Composable
internal fun rememberSettingsSelections(
    settings: List<AndroidSetting>,
    initialSelections: List<Int>,
) = remember(settings, initialSelections) {
    mutableStateListOf<Int>().apply {
        settings.forEachIndexed { index, setting ->
            val maxIndex =
                if (setting.kind == "range") {
                    (setting.rangeMax - setting.rangeMin).coerceAtLeast(0)
                } else {
                    max(0, setting.choices.lastIndex)
                }
            add(initialSelections.getOrElse(index) { 0 }.coerceIn(0, maxIndex))
        }
    }
}

@Composable
private fun DialogSettingButton(
    label: String,
    value: String,
    key: String,
    onClick: () -> Unit,
) {
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .clickable(onClick = onClick)
                .padding(vertical = 16.dp)
                .semantics { contentDescription = "$key: $label: $value" },
        horizontalArrangement = Arrangement.SpaceBetween,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(text = label, modifier = Modifier.weight(1f))
        Text(text = value, style = MaterialTheme.typography.bodyMedium)
    }
}

@Composable
private fun DialogChoiceButton(
    label: String,
    selected: Boolean,
    onClick: () -> Unit,
) {
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .clickable(onClick = onClick)
                .padding(vertical = 12.dp)
                .semantics { contentDescription = label },
        horizontalArrangement = Arrangement.SpaceBetween,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(text = label, modifier = Modifier.weight(1f))
        RadioButton(selected = selected, onClick = null)
    }
}

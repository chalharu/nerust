package io.github.chalharu.nerust

import android.os.Build
import android.util.Log
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
import androidx.compose.foundation.lazy.LazyListScope
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
import androidx.compose.material3.OutlinedTextField
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
import androidx.compose.ui.semantics.hideFromAccessibility
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.compose.ui.res.painterResource
import kotlin.math.roundToLong
import org.json.JSONArray
import org.json.JSONObject

/** One pick-list entry: the value rides back to Rust by index, so Kotlin
 * only needs the label plus whether the choice is currently selectable. */
internal data class SettingsChoice(
    val label: String,
    val enabled: Boolean = true,
    val reason: String = "",
)

/** Mirror the draft orientation choice onto the host activity: every
 * refreshed payload carries the live draft, so each tap rotates immediately
 * without waiting for save. Unknown indices are ignored (Rust owns 0/1/2). */
internal fun previewOrientation(
    sections: List<SettingsSection>,
    onOrientationChanged: (Int) -> Unit,
) {
    val row =
        sections
            .asSequence()
            .flatMap { it.rows }
            .firstOrNull { it.key == "video.orientation" } ?: return
    val orientation = screenOrientationRequest(row.selected) ?: return
    onOrientationChanged(orientation)
}

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
    val description: String = "",
    val detailTarget: String = "",
)

/** One settings section as projected by Rust (`settings_views::SectionWire`). */
internal data class SettingsSection(
    val id: String,
    val label: String,
    val rows: List<SettingsRow>,
    val visible: Boolean = true,
    val sub: String = "",
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
            val options = row.getJSONArray("choices")
            for (k in 0 until options.length()) {
                val option = options.getJSONObject(k)
                choices +=
                    SettingsChoice(
                        label = option.getString("label"),
                        enabled = option.getBoolean("enabled"),
                        reason = option.getString("reason"),
                    )
            }
            val kind = row.getString("kind")
            if (kind !in setOf("choice", "toggle", "number", "info", "header", "group", "capture", "button")) {
                Log.w(TAG, "unknown settings row kind: $kind (key=${row.getString("key")})")
            }
            rows +=
                SettingsRow(
                    key = row.getString("key"),
                    label = row.getString("label"),
                    kind = kind,
                    choices = choices,
                    selected = row.getInt("selected"),
                    number = row.getLong("number"),
                    step = row.getLong("step").coerceAtLeast(1L),
                    min = row.getLong("min"),
                    max = row.getLong("max"),
                    unit = row.getString("unit"),
                    text = row.getString("text"),
                    target = row.getString("target"),
                    visible = row.getBoolean("visible"),
                    description = row.getString("description"),
                    detailTarget = row.getString("detail_target"),
                )
        }
        val id = section.getString("id")
        val label = section.getString("label")
        val visible = section.getBoolean("visible")
        val sub = section.getString("sub")
        out += SettingsSection(id = id, label = label, rows = rows.filter { it.visible }, visible = visible, sub = sub)
    }
    return out
}

/** Single command builder: `payload` is op-specific JSON (`keyboards` uses
 * an array, `touch` an object, `set` omits it). One function, not two
 * overloads: Any? covers both shapes without duplicating the envelope. */
internal fun settingsCommandJson(
    key: String,
    index: Int,
    number: Long,
    op: String = "set",
    payload: Any? = null,
): String {
    val doc =
        JSONObject()
            .put("key", key)
            .put("index", index)
            .put("number", number)
            .put("op", op)
    if (payload != null) doc.put("payload", payload)
    return doc.toString()
}

/** Cross-boundary navigation actions the activity needs for system back,
 * discard confirmation, hardware-key/capture payload delivery, and
 * out-of-band refreshes (hotplug) that must not mark the draft dirty. */
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
    val refreshPayload: (String) -> Unit,
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
                title = stringResource(R.string.settings_error_title),
                buttons = {
                    TextButton(onClick = onDismissRequest) {
                        Text(stringResource(R.string.settings_ok))
                    }
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
    title: String,
    onLabel: String,
    offLabel: String,
): String =
    buildList {
        add(title)
        sections
            .filter { it.visible }
            .flatMap { it.rows }
            .filter { it.kind != "header" && it.kind != "button" }
            .forEach { row ->
                add("${row.label}: ${settingsRowDisplayValue(row, onLabel, offLabel)}")
            }
    }.joinToString("\n")

internal fun settingsRowDisplayValue(
    row: SettingsRow,
    onLabel: String,
    offLabel: String,
): String =
    when (row.kind) {
        "choice" -> row.choices.getOrNull(row.selected)?.label ?: "?"
        "toggle" -> row.choices.getOrNull(row.selected)?.label ?: if (row.selected == 1) onLabel else offLabel
        "number" -> "${row.number}${row.unit}"
        else -> row.text
    }

/** Fallback when a settings command fails before Rust answers. */
private const val TAG = "SettingsDialog"
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
    // Live orientation preview: the draft's video.orientation choice (0/1/2
    // in Rust build_play_section order) rides every refreshed payload, so
    // the screen rotates on tap like the language text switches on tap.
    // Save persists through the existing validated path; dismiss touches
    // nothing so the game screen keeps its saved lock.
    onOrientationChanged: (Int) -> Unit = {},
) {
    var sections by remember(initialSections) { mutableStateOf(initialSections) }
    var issues by remember(initialIssues) { mutableStateOf(initialIssues) }
    var capturePrompt by remember(initialCapture) { mutableStateOf(initialCapture) }
    // Navigation stack of section ids: pushing drills down one level,
    // popping returns exactly one level. The old single-id model jumped
    // from deep screens (device detail) straight to the top.
    var backStack by remember { mutableStateOf(listOf<String>()) }
    var activeRowKey by remember { mutableStateOf<String?>(null) }
    var commandError by remember { mutableStateOf<String?>(null) }
    var confirmDiscard by remember { mutableStateOf(false) }
    var isDirty by remember { mutableStateOf(false) }
    // Row keys touched since open: per-section unsaved dots. Reset-all
    // marks every row; a screen reset marks that screen's rows.
    var dirtyKeys by remember { mutableStateOf(setOf<String>()) }
    var pendingResetKey by remember { mutableStateOf<String?>(null) }

    fun applyPayloadJson(
        raw: String,
        mutating: Boolean,
    ): Boolean {
        try {
            val doc = JSONObject(raw)
            if (!doc.optBoolean("ok", false)) {
                Log.e(TAG, "settings command rejected: ${doc.optString("error", COMMAND_FAILED)}")
                commandError = doc.optString("error", COMMAND_FAILED)
                return false
            }
            val payload = doc.getJSONObject("payload")
            commandError = null
            sections = parseSettingsSections(payload)
            issues = parseIssues(payload)
            capturePrompt = parseCapture(payload)
            previewOrientation(sections, onOrientationChanged)
            if (mutating) isDirty = true
            return true
        } catch (e: Exception) {
            Log.e(TAG, "settings payload parse failed", e)
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
        payload: JSONArray? = null,
    ) {
        val raw =
            try {
                onCommand(settingsCommandJson(key, index, number, op, payload))
                    ?: throw IllegalStateException("settings bridge returned null")
            } catch (e: Exception) {
                Log.e(TAG, "settings bridge call failed", e)
                commandError = e.message ?: COMMAND_FAILED
                return
            }
        if (!applyPayloadJson(raw, mutating)) return
        if (!mutating) return
        dirtyKeys = resetDirtyKeys(sections, dirtyKeys, op, key)
    }

    val activeSection = backStack.lastOrNull()?.let { id -> sections.find { it.id == id } }
    val activeRow = activeSection?.rows?.find { it.key == activeRowKey }
    val canNavigateUp = activeSection != null || activeRow != null
    val isCapturing = capturePrompt != null

    fun pushSection(id: String) {
        activeRowKey = null
        if (backStack.lastOrNull() != id) backStack = backStack + id
    }

    fun navigateUp() {
        if (activeRow != null) {
            activeRowKey = null
        } else if (backStack.isNotEmpty()) {
            activeRowKey = null
            backStack = backStack.dropLast(1)
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
                    applyPayloadJson(raw, mutating = true)
                },
                refreshPayload = { raw ->
                    applyPayloadJson(raw, mutating = false)
                },
            ),
        )
    }

    val screenTitle = activeRow?.label ?: activeSection?.label ?: stringResource(R.string.settings_title)
    val subtitle = screenSubtitle(activeSection, activeRow)

    // Child-section parent map, shared by search breadcrumbs and
    // unsaved-dot ancestor propagation.
    val parents = remember(sections) { parentMap(sections) }
    val dirtyVisibleSections =
        remember(sections, dirtyKeys) { ancestorDirtySections(sections, dirtyKeys, parents) }
    val actions =
        SettingsRowActions(
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
                            pushSection(row.target)
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
            onDetailClick = { row ->
                if (row.detailTarget.isNotBlank() &&
                    sections.any { it.id == row.detailTarget }
                ) {
                    pushSection(row.detailTarget)
                }
            },
            onSectionClick = { pushSection(it) },
            onResetRequest = { pendingResetKey = it },
            onSearchNavigate = { sectionId, rowKey ->
                pushSection(sectionId)
                activeRowKey = rowKey
            },
        )
    val state =
        SettingsScreenState(
            sections = sections,
            activeSection = activeSection,
            activeRow = activeRow,
            issues = issues,
            capturePrompt = capturePrompt,
            commandError = commandError,
            parents = parents,
            dirtySections = dirtyVisibleSections,
        )

    NerustExpressiveHost {
        Scaffold(
            topBar = {
                SettingsTopBar(
                    title = screenTitle,
                    subtitle = subtitle,
                    canNavigateUp = canNavigateUp,
                    canSave = isDirty,
                    onNavigateUp = ::navigateUp,
                    onCancel = ::requestCancel,
                    onSave = onSave,
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
                state = state,
                actions = actions,
            )
        }
        SettingsExitDialogs(
            confirmDiscard = confirmDiscard,
            resetKey = pendingResetKey,
            onConfirmDiscard = {
                confirmDiscard = false
                onDismissRequest()
            },
            onDismissDiscard = { confirmDiscard = false },
            onConfirmReset = { key ->
                pendingResetKey = null
                sendCommand(key, 0, 0L, "reset_section")
            },
            onDismissReset = { pendingResetKey = null },
        )
    }
}

@Composable
private fun SettingsTopBar(
    title: String,
    subtitle: String?,
    canNavigateUp: Boolean,
    canSave: Boolean,
    onNavigateUp: () -> Unit,
    onCancel: () -> Unit,
    onSave: () -> Unit,
) {
    TopAppBar(
        title = {
            Column {
                Text(text = title)
                subtitle?.let {
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
                IconButton(onClick = onNavigateUp) {
                    Icon(
                        painter = painterResource(id = R.drawable.ic_settings_back),
                        contentDescription = stringResource(R.string.settings_back),
                    )
                }
            } else {
                IconButton(onClick = onCancel) {
                    Icon(
                        painter = painterResource(id = R.drawable.ic_settings_close),
                        contentDescription = stringResource(R.string.settings_close),
                    )
                }
            }
        },
        actions = {
            FilledTonalButton(onClick = onSave, enabled = canSave) {
                Text(stringResource(R.string.settings_save))
            }
        },
        colors =
            TopAppBarDefaults.topAppBarColors(
                containerColor = MaterialTheme.colorScheme.surfaceContainer,
            ),
    )
}

@Composable
private fun SettingsExitDialogs(
    confirmDiscard: Boolean,
    resetKey: String?,
    onConfirmDiscard: () -> Unit,
    onDismissDiscard: () -> Unit,
    onConfirmReset: (String) -> Unit,
    onDismissReset: () -> Unit,
) {
    if (confirmDiscard) {
        SettingsConfirmDialog(
            title = stringResource(R.string.settings_discard_title),
            text = stringResource(R.string.settings_discard_message),
            confirmLabel = stringResource(R.string.settings_discard_confirm),
            onConfirm = onConfirmDiscard,
            onDismiss = onDismissDiscard,
        )
    }
    resetKey?.let { key ->
        val isAll = key == "reset.all"
        SettingsConfirmDialog(
            title =
                stringResource(
                    if (isAll) {
                        R.string.settings_reset_all_title
                    } else {
                        R.string.settings_reset_screen_title
                    },
                ),
            text =
                stringResource(
                    if (isAll) {
                        R.string.settings_reset_all_message
                    } else {
                        R.string.settings_reset_screen_message
                    },
                ),
            confirmLabel = stringResource(R.string.settings_reset_confirm),
            onConfirm = { onConfirmReset(key) },
            onDismiss = onDismissReset,
        )
    }
}

@Composable
private fun SettingsConfirmDialog(
    title: String,
    text: String,
    confirmLabel: String,
    onConfirm: () -> Unit,
    onDismiss: () -> Unit,
) {
    androidx.compose.material3.AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(text = title) },
        text = { Text(text = text) },
        confirmButton = {
            TextButton(onClick = onConfirm) { Text(text = confirmLabel) }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) {
                Text(stringResource(R.string.settings_keep_editing))
            }
        },
    )
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

private data class SettingsBanners(
    val capturePrompt: String?,
    val issues: List<String>,
    val commandError: String?,
)

@Composable
private fun CaptureBanner(
    prompt: String,
    onCancelCapture: () -> Unit,
) {
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

@Composable
private fun IssuesBanner(issues: List<String>) {
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

@Composable
private fun CommandErrorBanner(message: String) {
    Column(modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp)) {
        Text(
            text = message,
            style = MaterialTheme.typography.titleSmall,
            color = MaterialTheme.colorScheme.error,
        )
        Text(
            text = stringResource(R.string.settings_update_retry),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.error,
        )
    }
}

// Banners ride as the first lazy items so they scroll with the content
// instead of squeezing the list on small screens.
private fun androidx.compose.foundation.lazy.LazyListScope.settingsBannerItems(
    banners: SettingsBanners,
    onCancelCapture: () -> Unit,
) {
    banners.capturePrompt?.let { prompt ->
        item(key = "banner-capture") {
            CaptureBanner(prompt = prompt, onCancelCapture = onCancelCapture)
        }
    }
    if (banners.issues.isNotEmpty()) {
        item(key = "banner-issues") {
            IssuesBanner(issues = banners.issues)
        }
    }
    if (banners.commandError != null) {
        item(key = "banner-error") {
            CommandErrorBanner(message = banners.commandError)
        }
    }
}

// Search results ride the lazy DSL like banners: the caller resolves
// labels up front, this only emits items.
private fun LazyListScope.settingsSearchResults(
    hits: List<Pair<SettingsSection, SettingsRow>>,
    emptyText: String,
    breadcrumbOf: (String) -> String,
    onNavigate: (String, String?) -> Unit,
) {
    if (hits.isEmpty()) {
        item(key = "settings-search-empty") {
            Card(
                shape = RoundedCornerShape(28.dp),
                colors =
                    CardDefaults.cardColors(
                        containerColor = MaterialTheme.colorScheme.surfaceContainerHigh,
                    ),
                modifier = Modifier.fillMaxWidth(),
            ) {
                Text(
                    text = emptyText,
                    style = MaterialTheme.typography.bodyMedium,
                    modifier = Modifier.padding(20.dp),
                )
            }
        }
    } else {
        items(hits, key = { hit -> "search:${hit.first.id}:${hit.second.key}" }) { (section, row) ->
            val trail =
                listOf(breadcrumbOf(section.id), section.label)
                    .filter { it.isNotBlank() }
                    .joinToString(" › ")
            Card(
                onClick = {
                    when {
                        row.kind == "choice" -> onNavigate(section.id, row.key)
                        row.kind == "group" && row.target.isNotBlank() ->
                            onNavigate(row.target, null)
                        row.detailTarget.isNotBlank() ->
                            onNavigate(row.detailTarget, null)
                        else -> onNavigate(section.id, null)
                    }
                },
                shape = RoundedCornerShape(28.dp),
                colors =
                    CardDefaults.cardColors(
                        containerColor = MaterialTheme.colorScheme.surfaceContainerHigh,
                    ),
                modifier = Modifier.fillMaxWidth(),
            ) {
                Column(modifier = Modifier.padding(20.dp)) {
                    Text(
                        text = row.label,
                        style = MaterialTheme.typography.titleMedium,
                    )
                    if (trail.isNotBlank()) {
                        Text(
                            text = trail,
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            }
        }
    }
}

/** Child section id -> (parent section id, parent label, via row label).
 * Group rows and device Details buttons both climb one level. Shared by
 * search breadcrumbs and unsaved-dot ancestor propagation. */
private fun parentMap(sections: List<SettingsSection>): Map<String, Triple<String, String, String>> =
    buildMap {
        for (section in sections) {
            for (row in section.rows) {
                val child =
                    when {
                        row.kind == "group" && row.target.isNotBlank() -> row.target
                        row.detailTarget.isNotBlank() -> row.detailTarget
                        else -> continue
                    }
                if (sections.any { it.id == child }) {
                    put(child, Triple(section.id, section.label, row.label))
                }
            }
        }
    }

/** Screen data resolved by the dialog root: Rust sections plus derived
 * navigation state. One object instead of N params (keeps S107 quiet). */
private data class SettingsScreenState(
    val sections: List<SettingsSection>,
    val activeSection: SettingsSection?,
    val activeRow: SettingsRow?,
    val issues: List<String>,
    val capturePrompt: String?,
    val commandError: String?,
    val parents: Map<String, Triple<String, String, String>>,
    val dirtySections: Set<String>,
)

/** Every row-level action, threaded from the dialog root to each list
 * level. One bundle instead of N callback params (keeps S107 quiet). */
private data class SettingsRowActions(
    val onCancelCapture: () -> Unit,
    val onSelectChoice: (SettingsRow, Int) -> Unit,
    val onToggle: (SettingsRow, Boolean) -> Unit,
    val onCommitNumber: (SettingsRow, Long) -> Unit,
    val onRowClick: (SettingsRow) -> Unit,
    val onClearCapture: (SettingsRow) -> Unit,
    val onDetailClick: (SettingsRow) -> Unit,
    val onSectionClick: (String) -> Unit,
    val onResetRequest: (String) -> Unit,
    val onSearchNavigate: (String, String?) -> Unit,
)

/** Reset dirty-marking: reset-all marks every row key, a screen reset
 * marks that screen's rows, a plain edit marks its own key. */
private fun resetDirtyKeys(
    sections: List<SettingsSection>,
    dirtyKeys: Set<String>,
    op: String,
    key: String,
): Set<String> =
    when {
        op == "reset_section" && key == "reset.all" ->
            sections.flatMap { section -> section.rows.map { it.key } }.toSet()
        op == "reset_section" ->
            sections
                .find { it.id == key.removePrefix("reset.") }
                ?.rows
                ?.map { it.key }
                .orEmpty()
                .toSet() + dirtyKeys
        key.isNotBlank() -> dirtyKeys + key
        else -> dirtyKeys
    }

/** Dirty sections plus every ancestor up to the visible categories,
 * so a deep edit still dots its top-level card. */
private fun ancestorDirtySections(
    sections: List<SettingsSection>,
    dirtyKeys: Set<String>,
    parents: Map<String, Triple<String, String, String>>,
): Set<String> {
    val direct =
        sections
            .filter { section ->
                section.rows.any { it.key in dirtyKeys }
            }.map { it.id }
            .toMutableSet()
    val queue = ArrayDeque(direct)
    while (queue.isNotEmpty()) {
        parents[queue.removeFirst()]?.let { (parentId, _, _) ->
            if (direct.add(parentId)) queue.add(parentId)
        }
    }
    return direct
}

/** Breadcrumb parent: a section's own sub (player name, controller root),
 * falling back to the section label under a choice row and to nothing
 * on the top level. */
private fun screenSubtitle(
    activeSection: SettingsSection?,
    activeRow: SettingsRow?,
): String? =
    activeSection?.let { section ->
        when {
            section.sub.isNotBlank() -> section.sub
            activeRow != null -> section.label
            else -> null
        }
    }

/** Breadcrumb chain for a hidden section: parent labels up to the
 * visible category, shared by search hits. */
private fun breadcrumbTrail(
    parents: Map<String, Triple<String, String, String>>,
    sectionId: String,
): String {
    val chain = mutableListOf<String>()
    var current: String? = sectionId
    while (current != null) {
        val parent = parents[current] ?: break
        chain.add(0, parent.second)
        current = parent.first
    }
    return chain.joinToString(" › ")
}

/** Search hits across every section (hidden ones included): label,
 * description, section label, value, or current text match. */
private fun searchHits(
    sections: List<SettingsSection>,
    query: String,
    onLabel: String,
    offLabel: String,
): List<Pair<SettingsSection, SettingsRow>> {
    if (query.isBlank()) return emptyList()
    return sections.flatMap { section ->
        section.rows
            .filter { row ->
                row.kind != "header" &&
                    (
                        row.label.contains(query, ignoreCase = true) ||
                            row.description.contains(query, ignoreCase = true) ||
                            section.label.contains(query, ignoreCase = true) ||
                            row.text.contains(query, ignoreCase = true) ||
                            settingsRowDisplayValue(row, onLabel, offLabel)
                                .contains(query, ignoreCase = true)
                    )
            }.map { row -> section to row }
    }
}

@Composable
private fun SettingsScreenContent(
    modifier: Modifier,
    state: SettingsScreenState,
    actions: SettingsRowActions,
) {
    val banners =
        remember(state.capturePrompt, state.issues, state.commandError) {
            SettingsBanners(
                capturePrompt = state.capturePrompt,
                issues = state.issues,
                commandError = state.commandError,
            )
        }
    Box(modifier = modifier) {
        when {
            state.activeRow != null && state.activeRow.kind == "choice" -> {
                SettingsChoiceList(
                    row = state.activeRow,
                    banners = banners,
                    onCancelCapture = actions.onCancelCapture,
                    onSelect = { choiceIndex -> actions.onSelectChoice(state.activeRow, choiceIndex) },
                )
            }

            state.activeSection != null -> {
                SettingsSectionList(
                    section = state.activeSection,
                    banners = banners,
                    actions = actions,
                )
            }

            else -> {
                SettingsCategoryList(
                    state = state,
                    banners = banners,
                    actions = actions,
                )
            }
        }
    }
}

@Composable
private fun SettingsCategoryList(
    state: SettingsScreenState,
    banners: SettingsBanners,
    actions: SettingsRowActions,
) {
    var query by remember { mutableStateOf("") }
    // Labels and hits resolve in composable context; the lazy content
    // below only consumes them through the plain list DSL.
    val onLabel = stringResource(R.string.settings_on)
    val offLabel = stringResource(R.string.settings_off)
    val hits = searchHits(state.sections, query, onLabel, offLabel)
    val emptyText =
        if (query.isBlank()) "" else stringResource(R.string.settings_search_empty, query)
    LazyColumn(
        modifier = Modifier.fillMaxSize(),
        verticalArrangement = Arrangement.spacedBy(12.dp),
        contentPadding = PaddingValues(vertical = 12.dp),
    ) {
        settingsBannerItems(banners = banners, onCancelCapture = actions.onCancelCapture)
        item(key = "settings-search") {
            OutlinedTextField(
                value = query,
                onValueChange = { query = it },
                modifier = Modifier.fillMaxWidth(),
                placeholder = { Text(stringResource(R.string.settings_search_hint)) },
                leadingIcon = {
                    Icon(
                        painter = painterResource(id = R.drawable.ic_settings_search),
                        contentDescription = null,
                    )
                },
                singleLine = true,
                shape = RoundedCornerShape(28.dp),
            )
        }
        if (query.isNotBlank()) {
            settingsSearchResults(
                hits = hits,
                emptyText = emptyText,
                breadcrumbOf = { sectionId -> breadcrumbTrail(state.parents, sectionId) },
                onNavigate = actions.onSearchNavigate,
            )
            return@LazyColumn
        }
        // Cards show visible categories only; search above covers hidden
        // screens through the same `sections` list.
        val visibleSections = state.sections.filter { it.visible }
        items(visibleSections, key = { section -> section.id }) { section ->
            // Summaries show inline-editable rows plus group rows that carry
            // a value (player/device summaries); bare chevrons stay out.
            // on/off labels ride the hoisted composable resolutions above.
            val summary =
                section.rows
                    .filter {
                        it.kind == "choice" || it.kind == "toggle" || it.kind == "number" ||
                            (it.kind == "group" && it.text.isNotBlank())
                    }.take(2)
                    .joinToString(" · ") { row ->
                        val value =
                            if (row.kind == "group") {
                                row.text
                            } else {
                                settingsRowDisplayValue(row, onLabel, offLabel)
                            }
                        "${row.label}: $value"
                    }
            // Headers group rows visually and buttons act; only real
            // entries count.
            val entryCount = section.rows.count { it.kind != "header" && it.kind != "button" }
            Card(
                onClick = { actions.onSectionClick(section.id) },
                shape = RoundedCornerShape(28.dp),
                colors =
                    CardDefaults.cardColors(
                        containerColor = MaterialTheme.colorScheme.surfaceContainerHigh,
                    ),
                modifier = Modifier.fillMaxWidth(),
            ) {
                Column(modifier = Modifier.padding(20.dp)) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Text(
                            text = section.label,
                            style = MaterialTheme.typography.titleLarge,
                            modifier = Modifier.weight(1f),
                        )
                        if (section.id in state.dirtySections) {
                            Text(
                                text = "•",
                                style = MaterialTheme.typography.titleLarge,
                                color = MaterialTheme.colorScheme.error,
                                modifier = Modifier.semantics { hideFromAccessibility() },
                            )
                        }
                    }
                    Text(
                        text =
                            pluralStringResource(
                                R.plurals.settings_count,
                                entryCount,
                                entryCount,
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
    banners: SettingsBanners,
    actions: SettingsRowActions,
) {
    // Split rows into groups at header rows; each group renders as one card.
    val groups = remember(section) { splitSectionGroups(section.rows) }
    val onLabel = stringResource(R.string.settings_on)
    val offLabel = stringResource(R.string.settings_off)
    LazyColumn(
        modifier = Modifier.fillMaxSize(),
        verticalArrangement = Arrangement.spacedBy(12.dp),
        contentPadding = PaddingValues(vertical = 12.dp),
    ) {
        settingsBannerItems(banners = banners, onCancelCapture = actions.onCancelCapture)
        groups.forEachIndexed { groupIndex, group ->
            if (group.title.isNotBlank()) {
                item(key = "${section.id}#$groupIndex") {
                    Text(
                        text = group.title,
                        style = MaterialTheme.typography.titleSmall,
                        color = MaterialTheme.colorScheme.primary,
                        modifier = Modifier.padding(start = 8.dp, top = 8.dp, bottom = 4.dp),
                    )
                }
            }
            item(key = "${section.id}#$groupIndex#card") {
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
                                        onToggle = { actions.onToggle(row, it) },
                                        onDetail = { actions.onDetailClick(row) },
                                    )
                                }
                                "number" -> {
                                    SettingsSliderRow(
                                        row = row,
                                        onCommit = { actions.onCommitNumber(row, it) },
                                    )
                                }
                                "choice" -> {
                                    SettingsNavigationRow(
                                        title = row.label,
                                        value = settingsRowDisplayValue(row, onLabel, offLabel),
                                        semanticsLabel = "${row.label}: ${settingsRowDisplayValue(row, onLabel, offLabel)}",
                                        supportingText = row.description,
                                        onClick = { actions.onRowClick(row) },
                                    )
                                }
                                "group" -> {
                                    SettingsNavigationRow(
                                        title = row.label,
                                        value = row.text,
                                        semanticsLabel = row.label,
                                        supportingText = row.description,
                                        showChevron = true,
                                        onClick = { actions.onRowClick(row) },
                                    )
                                }
                                "button" -> {
                                    SettingsButtonRow(
                                        row = row,
                                        onRun = { actions.onResetRequest(row.key) },
                                    )
                                }
                                "capture" -> {
                                    SettingsCaptureRow(
                                        row = row,
                                        onStart = { actions.onRowClick(row) },
                                        onClear = { actions.onClearCapture(row) },
                                    )
                                }
                                else -> {
                                    SettingsInfoRow(
                                        title = row.label,
                                        value = settingsRowDisplayValue(row, onLabel, offLabel),
                                        semanticsLabel = row.label,
                                        supportingText = row.description,
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
    onDetail: () -> Unit,
) {
    val checked = row.selected == 1
    // The whole toggle area flips the value: the inner row owns one
    // toggleable (label, description, switch state announced as a single
    // switch), the trailing Details button stays a sibling so TalkBack
    // keeps it separately activatable.
    Row(
        modifier = Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Row(
            modifier =
                Modifier
                    .weight(1f)
                    .toggleable(
                        value = checked,
                        role = Role.Switch,
                        onValueChange = onToggle,
                    )
                    .padding(horizontal = 24.dp, vertical = 12.dp)
                    .semantics(mergeDescendants = true) {},
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Column(modifier = Modifier.weight(1f)) {
                Text(
                    text = row.label,
                    style = MaterialTheme.typography.titleMedium,
                )
                if (row.description.isNotBlank()) {
                    Text(
                        text = row.description,
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
            // Indicator only: the row owns the toggle so a tap on either
            // end fires once.
            Switch(
                checked = checked,
                onCheckedChange = null,
            )
        }
        if (row.detailTarget.isNotBlank()) {
            TextButton(onClick = onDetail) {
                Text(stringResource(R.string.settings_details))
            }
        }
    }
}

@Composable
private fun SettingsButtonRow(
    row: SettingsRow,
    onRun: () -> Unit,
) {
    // A single full-width action: label plus supporting text, no chevron,
    // no value. Kotlin confirms first; Rust restores defaults on confirm.
    TextButton(
        onClick = onRun,
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(horizontal = 12.dp, vertical = 4.dp),
    ) {
        Column(
            modifier = Modifier.fillMaxWidth(),
            horizontalAlignment = Alignment.Start,
        ) {
            Text(
                text = row.label,
                style = MaterialTheme.typography.titleMedium,
            )
            if (row.description.isNotBlank()) {
                Text(
                    text = row.description,
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
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
                .padding(horizontal = 24.dp, vertical = 12.dp),
    ) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Column(modifier = Modifier.weight(1f)) {
                Text(
                    text = row.label,
                    style = MaterialTheme.typography.titleMedium,
                )
                if (row.description.isNotBlank()) {
                    Text(
                        text = row.description,
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
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
    // Clear tap also start a capture. No container label: each sibling
    // speaks for itself, so TalkBack never hears the row twice.
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(horizontal = 24.dp, vertical = 12.dp),
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
    banners: SettingsBanners,
    onCancelCapture: () -> Unit,
    onSelect: (Int) -> Unit,
) {
    LazyColumn(
        modifier = Modifier.fillMaxSize(),
        verticalArrangement = Arrangement.spacedBy(12.dp),
        contentPadding = PaddingValues(vertical = 12.dp),
    ) {
        settingsBannerItems(banners = banners, onCancelCapture = onCancelCapture)
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
                        // Rust passes the owner slot label; Kotlin owns the sentence.
                        val reason =
                            choice.reason.ifBlank { null }?.let { owner ->
                                stringResource(R.string.settings_used_in, owner)
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
                                    .semantics(mergeDescendants = true) {},
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
                .semantics(mergeDescendants = true) { contentDescription = semanticsLabel },
        horizontalArrangement = Arrangement.SpaceBetween,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(modifier = Modifier.weight(1f)) {
            Text(text = title, style = MaterialTheme.typography.titleMedium)
            supportingText?.takeIf(String::isNotBlank)?.let {
                Text(
                    text = it,
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
        if (value.isNotBlank()) {
            Text(text = value, style = MaterialTheme.typography.bodyMedium)
        }
        if (showChevron) {
            Icon(
                painter = painterResource(id = R.drawable.ic_settings_chevron),
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
    supportingText: String? = null,
) {
    // Static readout: deliberately not clickable so it never pushes a
    // detail level that renders nothing.
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(horizontal = 24.dp, vertical = 18.dp)
                .semantics(mergeDescendants = true) { contentDescription = semanticsLabel },
        horizontalArrangement = Arrangement.SpaceBetween,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(modifier = Modifier.weight(1f)) {
            Text(text = title, style = MaterialTheme.typography.titleMedium)
            supportingText?.takeIf(String::isNotBlank)?.let {
                Text(
                    text = it,
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
        Text(text = value, style = MaterialTheme.typography.bodyMedium)
    }
}


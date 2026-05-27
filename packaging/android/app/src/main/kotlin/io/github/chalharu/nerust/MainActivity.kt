package io.github.chalharu.nerust

import android.app.AlertDialog
import android.app.NativeActivity
import android.content.Intent
import android.os.Bundle
import android.util.Log
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.widget.ArrayAdapter
import android.widget.FrameLayout
import android.widget.ListView
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.material3.DrawerValue
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalDrawerSheet
import androidx.compose.material3.ModalNavigationDrawer
import androidx.compose.material3.NavigationDrawerItem
import androidx.compose.material3.NavigationDrawerItemDefaults
import androidx.compose.material3.Text
import androidx.compose.material3.rememberDrawerState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.ComposeView
import androidx.compose.ui.platform.ViewCompositionStrategy
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch

private const val DRAWER_OVERLAY_TAG = "nerust-drawer-overlay"
private const val MENU_ACTION_LOAD_STATE = "load_state"
private const val MENU_ACTION_OPEN_LIBRARY = "open_library"
private const val MENU_ACTION_OPEN_SETTINGS = "open_settings"
private const val MENU_ACTION_RESET = "reset"
private const val MENU_ACTION_SAVE_STATE = "save_state"
private const val MENU_ACTION_TOGGLE_PAUSE = "toggle_pause"
private const val MENU_BUTTON_TAG = "nerust-menu-button"

class MainActivity : NativeActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.decorView.post(::ensureMenuChromeAttached)
    }

    override fun onResume() {
        super.onResume()
        window.decorView.post(::ensureMenuChromeAttached)
    }

    @Suppress("DEPRECATION")
    override fun onBackPressed() {
        if (removeDrawerOverlay()) {
            return
        }
        super.onBackPressed()
    }

    @Suppress("DEPRECATION")
    fun startRomPicker() {
        val intent = Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
            addCategory(Intent.CATEGORY_OPENABLE)
            type = "*/*"
        }
        startActivityForResult(intent, ROM_PICKER_REQUEST_CODE)
    }

    @Suppress("DEPRECATION")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode != ROM_PICKER_REQUEST_CODE) {
            return
        }

        val uri = if (resultCode == RESULT_OK) data?.data else null
        if (uri != null) {
            val takeFlags = data?.flags?.and(
                Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION,
            ) ?: Intent.FLAG_GRANT_READ_URI_PERMISSION
            try {
                contentResolver.takePersistableUriPermission(uri, takeFlags)
            } catch (error: SecurityException) {
                Log.w(TAG, "Failed to keep Android ROM URI permission", error)
            }
        }

        onFilePickerResult(uri?.toString())
    }

    /**
     * Show a modal ROM library dialog.
     *
     * The first item is always "Import new ROM…"; the remaining items are the
     * provided library entries in order.  When the user makes a selection this
     * method calls [onRomLibrarySelected] with the appropriate id and then
     * returns control to Rust.  On cancel/dismiss it calls
     * [onRomLibrarySelected] with `null`.
     *
     * Called from the Rust JNI bridge on the Java main thread.
     */
    fun showRomLibraryDialog(entryNames: Array<String>, entryIds: Array<String>) {
        val items = ArrayList<String>(entryNames.size + 1)
        items.add("Import new ROM\u2026")
        items.addAll(entryNames)
        var resultSent = false

        AlertDialog.Builder(this)
            .setTitle("ROM Library")
            .setItems(items.toTypedArray()) { _, which ->
                resultSent = true
                if (which == 0) {
                    // User wants to import – tell Rust first, which will
                    // trigger the SAF picker on its own next event-loop turn.
                    onRomLibrarySelected(IMPORT_ACTION_ID)
                } else {
                    onRomLibrarySelected(entryIds[which - 1])
                }
            }
            .setOnDismissListener {
                if (!resultSent) {
                    onRomLibrarySelected(null)
                }
            }
            .show()
    }

    /**
     * Show a modal Android settings dialog.
     *
     * Presents an Android-relevant subset of settings. Each setting is backed
     * by a tab-separated list of choices; the current selection is identified
     * by index.  Tapping a row in the list opens a single-choice sub-dialog.
     * Tapping "Save" calls [onSettingsDialogResult] with a comma-separated
     * string of the final choice indices.  Cancel/dismiss calls it with `null`.
     *
     * @param keys           Stable setting identifiers (kept for documentation
     *                       clarity and potential future use).
     * @param labels         Human-readable label for each setting.
     * @param choiceStrings  Tab-separated choice labels for each setting.
     * @param currentIndices Current choice index as a string for each setting.
     *
     * Called from the Rust JNI bridge on the Java main thread.
     */
    fun showSettingsDialog(
        keys: Array<String>,
        labels: Array<String>,
        choiceStrings: Array<String>,
        currentIndices: Array<String>,
    ) {
        val selectedIndices = IntArray(labels.size) { i ->
            currentIndices.getOrNull(i)?.toIntOrNull() ?: 0
        }
        val choiceArrays: Array<Array<String>> = Array(choiceStrings.size) { i ->
            choiceStrings[i].split('\t').toTypedArray()
        }

        fun itemText(i: Int): String =
            "${labels[i]}: ${choiceArrays[i].getOrElse(selectedIndices[i]) { "?" }}"

        val displayItems = Array(labels.size) { i -> itemText(i) }
        val listView = ListView(this)
        val adapter = ArrayAdapter(this, android.R.layout.simple_list_item_1, displayItems)
        listView.adapter = adapter

        var resultSent = false

        val parentDialog = AlertDialog.Builder(this)
            .setTitle("Settings")
            .setView(listView)
            .setPositiveButton("Save") { _, _ ->
                resultSent = true
                onSettingsDialogResult(selectedIndices.joinToString(","))
            }
            .setNegativeButton("Cancel") { _, _ ->
                resultSent = true
                onSettingsDialogResult(null)
            }
            .setOnDismissListener {
                if (!resultSent) {
                    onSettingsDialogResult(null)
                }
            }
            .create()

        listView.setOnItemClickListener { _, _, which, _ ->
            AlertDialog.Builder(this)
                .setTitle(labels[which])
                .setSingleChoiceItems(choiceArrays[which], selectedIndices[which]) { subDialog, choice ->
                    selectedIndices[which] = choice
                    displayItems[which] = itemText(which)
                    adapter.notifyDataSetChanged()
                    subDialog.dismiss()
                }
                .setNegativeButton("Cancel", null)
                .show()
        }

        parentDialog.show()
    }

    private fun ensureMenuChromeAttached() {
        val root = contentRoot() ?: return
        val button = root.findViewWithTag<View>(MENU_BUTTON_TAG) ?: createMenuButtonOverlay().also(root::addView)
        button.bringToFront()
        root.findViewWithTag<View>(DRAWER_OVERLAY_TAG)?.bringToFront()
    }

    private fun createMenuButtonOverlay(): ComposeView =
        ComposeView(this).apply {
            tag = MENU_BUTTON_TAG
            layoutParams = FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.WRAP_CONTENT,
                ViewGroup.LayoutParams.WRAP_CONTENT,
                Gravity.TOP or Gravity.START,
            )
            setViewCompositionStrategy(ViewCompositionStrategy.DisposeOnDetachedFromWindow)
            setContent {
                MaterialTheme {
                    NerustMenuButton(onOpenMenu = ::showDrawerOverlay)
                }
            }
        }

    private fun showDrawerOverlay() {
        val root = contentRoot() ?: return
        val existing = root.findViewWithTag<View>(DRAWER_OVERLAY_TAG)
        if (existing != null) {
            existing.bringToFront()
            return
        }

        val overlay = ComposeView(this).apply {
            tag = DRAWER_OVERLAY_TAG
            layoutParams = FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.MATCH_PARENT,
            )
            setViewCompositionStrategy(ViewCompositionStrategy.DisposeOnDetachedFromWindow)
            setContent {
                MaterialTheme {
                    NerustDrawerOverlay(
                        onDismissRequest = { removeDrawerOverlay() },
                        onMenuAction = ::dispatchMenuAction,
                    )
                }
            }
        }
        root.addView(overlay)
        overlay.bringToFront()
    }

    private fun removeDrawerOverlay(): Boolean {
        val root = contentRoot() ?: return false
        val overlay = root.findViewWithTag<View>(DRAWER_OVERLAY_TAG) ?: return false
        root.removeView(overlay)
        return true
    }

    private fun contentRoot(): ViewGroup? = findViewById(android.R.id.content)

    private fun dispatchMenuAction(action: String) {
        onMenuAction(action)
    }

    private external fun onFilePickerResult(uri: String?)

    private external fun onMenuAction(action: String)

    private external fun onRomLibrarySelected(id: String?)

    private external fun onSettingsDialogResult(result: String?)

    companion object {
        private const val TAG = "Nerust"
        private const val ROM_PICKER_REQUEST_CODE = 0x4E45
        // Must match `android/library.rs::IMPORT_ACTION_ID`.
        private const val IMPORT_ACTION_ID = "__import__"
    }
}

@Composable
private fun NerustMenuButton(onOpenMenu: () -> Unit) {
    Box(
        modifier = Modifier
            .statusBarsPadding()
            .padding(16.dp),
    ) {
        FilledTonalButton(onClick = onOpenMenu) {
            Text("Menu")
        }
    }
}

@Composable
private fun NerustDrawerOverlay(
    onDismissRequest: () -> Unit,
    onMenuAction: (String) -> Unit,
) {
    val drawerState = rememberDrawerState(initialValue = DrawerValue.Open)
    val scope = rememberCoroutineScope()

    LaunchedEffect(drawerState.currentValue) {
        if (drawerState.currentValue == DrawerValue.Closed) {
            onDismissRequest()
        }
    }

    ModalNavigationDrawer(
        drawerState = drawerState,
        drawerContent = {
            ModalDrawerSheet(modifier = Modifier.statusBarsPadding()) {
                Text(
                    text = "Nerust",
                    style = MaterialTheme.typography.titleLarge,
                    modifier = Modifier.padding(start = 24.dp, top = 24.dp, end = 24.dp, bottom = 8.dp),
                )
                Text(
                    text = "Open ROMs and control the current session.",
                    style = MaterialTheme.typography.bodyMedium,
                    modifier = Modifier.padding(start = 24.dp, end = 24.dp, bottom = 16.dp),
                )
                DrawerActionItem(
                    label = "ROM Library",
                    onClick = {
                        scope.launch {
                            drawerState.close()
                            onMenuAction(MENU_ACTION_OPEN_LIBRARY)
                        }
                    },
                )
                DrawerActionItem(
                    label = "Settings",
                    onClick = {
                        scope.launch {
                            drawerState.close()
                            onMenuAction(MENU_ACTION_OPEN_SETTINGS)
                        }
                    },
                )
                HorizontalDivider(modifier = Modifier.padding(vertical = 8.dp))
                DrawerActionItem(
                    label = "Pause / Resume",
                    onClick = {
                        scope.launch {
                            drawerState.close()
                            onMenuAction(MENU_ACTION_TOGGLE_PAUSE)
                        }
                    },
                )
                DrawerActionItem(
                    label = "Save State",
                    onClick = {
                        scope.launch {
                            drawerState.close()
                            onMenuAction(MENU_ACTION_SAVE_STATE)
                        }
                    },
                )
                DrawerActionItem(
                    label = "Load State",
                    onClick = {
                        scope.launch {
                            drawerState.close()
                            onMenuAction(MENU_ACTION_LOAD_STATE)
                        }
                    },
                )
                DrawerActionItem(
                    label = "Reset",
                    onClick = {
                        scope.launch {
                            drawerState.close()
                            onMenuAction(MENU_ACTION_RESET)
                        }
                    },
                )
            }
        },
    ) {
        Box(
            modifier = Modifier
                .fillMaxSize()
                .padding(WindowInsets.safeDrawing.asPaddingValues()),
        )
    }
}

@Composable
private fun DrawerActionItem(label: String, onClick: () -> Unit) {
    NavigationDrawerItem(
        label = { Text(label) },
        selected = false,
        onClick = onClick,
        modifier = Modifier.padding(NavigationDrawerItemDefaults.ItemPadding),
    )
}

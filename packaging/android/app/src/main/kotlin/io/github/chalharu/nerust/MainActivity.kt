package io.github.chalharu.nerust

import android.app.AlertDialog
import android.app.NativeActivity
import android.content.Context
import android.content.Intent
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.RectF
import android.graphics.Typeface
import android.os.Bundle
import android.util.Log
import android.view.Gravity
import android.view.MotionEvent
import android.view.View
import android.view.ViewGroup
import android.widget.ArrayAdapter
import android.widget.Button
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
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.ComposeView
import androidx.compose.ui.platform.ViewCompositionStrategy
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.LifecycleRegistry
import androidx.lifecycle.ViewModelStore
import androidx.lifecycle.ViewModelStoreOwner
import androidx.lifecycle.setViewTreeLifecycleOwner
import androidx.lifecycle.setViewTreeViewModelStoreOwner
import androidx.savedstate.SavedStateRegistry
import androidx.savedstate.SavedStateRegistryController
import androidx.savedstate.SavedStateRegistryOwner
import androidx.savedstate.setViewTreeSavedStateRegistryOwner
import kotlin.math.max
import kotlin.math.min
import kotlinx.coroutines.launch

private const val CONTROLS_OVERLAY_TAG = "nerust-controls-overlay"
private const val DRAWER_OVERLAY_TAG = "nerust-drawer-overlay"
private const val MENU_ACTION_LOAD_STATE = "load_state"
private const val MENU_ACTION_OPEN_LIBRARY = "open_library"
private const val MENU_ACTION_OPEN_SETTINGS = "open_settings"
private const val MENU_ACTION_RESET = "reset"
private const val MENU_ACTION_SAVE_STATE = "save_state"
private const val MENU_ACTION_TOGGLE_PAUSE = "toggle_pause"
private const val MENU_BUTTON_TAG = "nerust-menu-button"

class MainActivity : NativeActivity(), LifecycleOwner, SavedStateRegistryOwner, ViewModelStoreOwner {
    private val lifecycleRegistry = LifecycleRegistry(this)
    private val registryController = SavedStateRegistryController.create(this)
    private val store = ViewModelStore()

    override val lifecycle: Lifecycle
        get() = lifecycleRegistry

    override val savedStateRegistry: SavedStateRegistry
        get() = registryController.savedStateRegistry

    override val viewModelStore: ViewModelStore
        get() = store

    override fun onCreate(savedInstanceState: Bundle?) {
        registryController.performAttach()
        registryController.performRestore(savedInstanceState)
        super.onCreate(savedInstanceState)
        lifecycleRegistry.handleLifecycleEvent(Lifecycle.Event.ON_CREATE)
        window.decorView.post(::ensureMenuChromeAttached)
    }

    override fun onStart() {
        super.onStart()
        lifecycleRegistry.handleLifecycleEvent(Lifecycle.Event.ON_START)
    }

    override fun onResume() {
        super.onResume()
        lifecycleRegistry.handleLifecycleEvent(Lifecycle.Event.ON_RESUME)
        window.decorView.post(::ensureMenuChromeAttached)
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (hasFocus) {
            window.decorView.post(::ensureMenuChromeAttached)
        }
    }

    override fun onPause() {
        lifecycleRegistry.handleLifecycleEvent(Lifecycle.Event.ON_PAUSE)
        super.onPause()
    }

    override fun onStop() {
        lifecycleRegistry.handleLifecycleEvent(Lifecycle.Event.ON_STOP)
        super.onStop()
    }

    override fun onSaveInstanceState(outState: Bundle) {
        super.onSaveInstanceState(outState)
        registryController.performSave(outState)
    }

    override fun onDestroy() {
        lifecycleRegistry.handleLifecycleEvent(Lifecycle.Event.ON_DESTROY)
        store.clear()
        super.onDestroy()
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
        installComposeOwners(root)
        val controls = root.findViewWithTag<View>(CONTROLS_OVERLAY_TAG)
            ?: createControlsOverlay().also(::addOverlayView)
        val button = root.findViewWithTag<View>(MENU_BUTTON_TAG)
            ?: createMenuButtonOverlay().also(::addOverlayView)
        controls.bringToFront()
        button.bringToFront()
        root.findViewWithTag<View>(DRAWER_OVERLAY_TAG)?.bringToFront()
    }

    private fun addOverlayView(view: View) {
        if (view.parent == null) {
            addContentView(view, view.layoutParams)
        }
    }

    private fun installComposeOwners(root: View) {
        listOf(window.decorView, root).forEach { view ->
            view.setViewTreeLifecycleOwner(this)
            view.setViewTreeSavedStateRegistryOwner(this)
            view.setViewTreeViewModelStoreOwner(this)
        }
    }

    private fun createControlsOverlay(): View =
        ControlsOverlayView(this).apply {
            tag = CONTROLS_OVERLAY_TAG
            layoutParams = FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.MATCH_PARENT,
            )
            isClickable = false
            isFocusable = false
            importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
        }

    private fun createMenuButtonOverlay(): Button =
        Button(this).apply {
            tag = MENU_BUTTON_TAG
            text = "Menu"
            contentDescription = "Menu"
            setAllCaps(false)
            setOnClickListener { showDrawerOverlay() }
            layoutParams = FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.WRAP_CONTENT,
                ViewGroup.LayoutParams.WRAP_CONTENT,
                Gravity.TOP or Gravity.START,
            ).apply {
                val margin = dp(16)
                leftMargin = margin
                topMargin = statusBarHeight() + margin
            }
            importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_YES
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
        addOverlayView(overlay)
        overlay.bringToFront()
    }

    private fun removeDrawerOverlay(): Boolean {
        val root = contentRoot() ?: return false
        val overlay = root.findViewWithTag<View>(DRAWER_OVERLAY_TAG) ?: return false
        root.removeView(overlay)
        return true
    }

    private fun contentRoot(): ViewGroup? =
        findViewById<View>(android.R.id.content) as? ViewGroup ?: window.decorView as? ViewGroup

    private fun dp(value: Int): Int =
        (value * resources.displayMetrics.density).toInt()

    private fun statusBarHeight(): Int {
        val resourceId = resources.getIdentifier("status_bar_height", "dimen", "android")
        return if (resourceId > 0) resources.getDimensionPixelSize(resourceId) else 0
    }

    private fun dispatchMenuAction(action: String) {
        removeDrawerOverlay()
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
private fun NerustDrawerOverlay(
    onDismissRequest: () -> Unit,
    onMenuAction: (String) -> Unit,
) {
    val drawerState = rememberDrawerState(initialValue = DrawerValue.Open)
    val actionPending = remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    fun closeAndRun(action: String) {
        actionPending.value = true
        scope.launch {
            drawerState.close()
            onMenuAction(action)
        }
    }

    LaunchedEffect(drawerState.currentValue, actionPending.value) {
        if (drawerState.currentValue == DrawerValue.Closed && !actionPending.value) {
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
                        closeAndRun(MENU_ACTION_OPEN_LIBRARY)
                    },
                )
                DrawerActionItem(
                    label = "Settings",
                    onClick = {
                        closeAndRun(MENU_ACTION_OPEN_SETTINGS)
                    },
                )
                HorizontalDivider(modifier = Modifier.padding(vertical = 8.dp))
                DrawerActionItem(
                    label = "Pause / Resume",
                    onClick = {
                        closeAndRun(MENU_ACTION_TOGGLE_PAUSE)
                    },
                )
                DrawerActionItem(
                    label = "Save State",
                    onClick = {
                        closeAndRun(MENU_ACTION_SAVE_STATE)
                    },
                )
                DrawerActionItem(
                    label = "Load State",
                    onClick = {
                        closeAndRun(MENU_ACTION_LOAD_STATE)
                    },
                )
                DrawerActionItem(
                    label = "Reset",
                    onClick = {
                        closeAndRun(MENU_ACTION_RESET)
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
        modifier = Modifier
            .semantics { contentDescription = label }
            .padding(NavigationDrawerItemDefaults.ItemPadding),
    )
}

private class ControlsOverlayView(context: Context) : View(context) {
    private val fillPaint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        color = Color.argb(48, 255, 255, 255)
        style = Paint.Style.FILL
    }
    private val strokePaint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        color = Color.argb(160, 255, 255, 255)
        strokeWidth = 2f
        style = Paint.Style.STROKE
    }
    private val textPaint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        color = Color.argb(220, 255, 255, 255)
        textAlign = Paint.Align.CENTER
        typeface = Typeface.create(Typeface.DEFAULT, Typeface.BOLD)
    }

    override fun onDraw(canvas: Canvas) {
        super.onDraw(canvas)
        val viewWidth = width.toFloat()
        val viewHeight = height.toFloat()
        if (viewWidth <= 0f || viewHeight <= 0f) {
            return
        }

        val controlTop = viewHeight * 0.52f
        val controlHeight = viewHeight - controlTop
        val dpadLeft = viewWidth * 0.06f
        val dpadSize = viewWidth * 0.30f
        val actionSize = viewWidth * 0.16f
        val actionRight = viewWidth * 0.76f
        val faceTop = controlTop + controlHeight * 0.10f
        val centerTop = controlTop + controlHeight * 0.38f

        drawZone(
            canvas,
            dpadLeft + dpadSize * 0.25f,
            controlTop + controlHeight * 0.05f,
            dpadSize * 0.50f,
            dpadSize * 0.28f,
            "UP",
        )
        drawZone(
            canvas,
            dpadLeft + dpadSize * 0.25f,
            controlTop + controlHeight * 0.47f,
            dpadSize * 0.50f,
            dpadSize * 0.28f,
            "DOWN",
        )
        drawZone(
            canvas,
            dpadLeft,
            controlTop + controlHeight * 0.26f,
            dpadSize * 0.28f,
            dpadSize * 0.36f,
            "LEFT",
        )
        drawZone(
            canvas,
            dpadLeft + dpadSize * 0.47f,
            controlTop + controlHeight * 0.26f,
            dpadSize * 0.28f,
            dpadSize * 0.36f,
            "RIGHT",
        )
        drawZone(canvas, actionRight - actionSize * 1.1f, faceTop + actionSize * 0.55f, actionSize, actionSize, "B")
        drawZone(canvas, actionRight, faceTop, actionSize, actionSize, "A")
        drawZone(canvas, viewWidth * 0.36f, centerTop, viewWidth * 0.12f, viewHeight * 0.05f, "SELECT")
        drawZone(canvas, viewWidth * 0.52f, centerTop, viewWidth * 0.12f, viewHeight * 0.05f, "START")
    }

    override fun onTouchEvent(event: MotionEvent): Boolean = false

    private fun drawZone(
        canvas: Canvas,
        x: Float,
        y: Float,
        width: Float,
        height: Float,
        label: String,
    ) {
        val rect = RectF(x, y, x + width, y + height)
        val radius = min(width, height) * 0.20f
        canvas.drawRoundRect(rect, radius, radius, fillPaint)
        canvas.drawRoundRect(rect, radius, radius, strokePaint)
        textPaint.textSize = max(12f, min(height * 0.42f, width * 0.28f))
        val centerY = rect.centerY() - (textPaint.descent() + textPaint.ascent()) / 2f
        canvas.drawText(label, rect.centerX(), centerY, textPaint)
    }
}

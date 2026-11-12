package io.github.chalharu.nerust

import android.content.Context
import android.content.Intent
import android.content.pm.ActivityInfo
import android.os.Bundle
import android.util.Log
import androidx.activity.ComponentActivity
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.ComposeView
import androidx.compose.ui.platform.ViewCompositionStrategy
import androidx.lifecycle.setViewTreeLifecycleOwner
import androidx.lifecycle.setViewTreeViewModelStoreOwner
import androidx.savedstate.setViewTreeSavedStateRegistryOwner

private const val EXTRA_SLOTS = "nerust.extra.SLOTS"
private const val EXTRA_ORIENTATION = "nerust.extra.ORIENTATION"

/** Standalone save-slot manager. MainActivity passes one slot list per
 * open; select/create/delete ride the same menu lane as the drawer.
 * Refresh is a Rust push: after a slot op lands, the bridge calls back
 * with the fresh list (see MainActivity.onSlotListChanged). No polling.
 * Being an Activity (not a Dialog window) it looks, navigates, and backs
 * out like the settings screen instead of a centered popup. */
class SlotActivity : ComponentActivity() {
    private var rows by mutableStateOf(emptyList<SlotEntry>())

    override fun attachBaseContext(newBase: Context) {
        super.attachBaseContext(AppLocale.wrap(newBase))
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // Same orientation lock as the settings screen: follow the game
        // screen instead of falling back to the system default.
        val extraOrientation =
            intent.getIntExtra(EXTRA_ORIENTATION, ActivityInfo.SCREEN_ORIENTATION_UNSPECIFIED)
        if (requestedOrientation != extraOrientation) {
            requestedOrientation = extraOrientation
        }
        val raw = intent.getStringExtra(EXTRA_SLOTS)
        if (raw == null) {
            Log.e(TAG, "SlotActivity: missing slot list")
            finish()
            return
        }
        rows = parseSlotList(raw)
        // The push lands on the main thread; a destroyed screen clears
        // the listener first, so no refresh ever hits a dead composition.
        MainActivity.slotListener = { json -> rows = parseSlotList(json) }
        val composeView =
            ComposeView(this).apply {
                setViewCompositionStrategy(ViewCompositionStrategy.DisposeOnDetachedFromWindow)
            }
        setContentView(composeView)
        listOf(window.decorView, composeView).forEach { view ->
            view.setViewTreeLifecycleOwner(this)
            view.setViewTreeSavedStateRegistryOwner(this)
            view.setViewTreeViewModelStoreOwner(this)
        }
        composeView.setContent {
            SlotMenuScreen(
                rows = rows,
                onSelect = { id ->
                    postSlotAction(
                        menuEvent(MenuAction.SELECT_SLOT) {
                            put("slot_id", id)
                        },
                    )
                    finish()
                },
                onDelete = { id ->
                    postSlotAction(
                        menuEvent(MenuAction.DELETE_SLOT) {
                            put("slot_id", id)
                        },
                    )
                },
                onCreate = {
                    postSlotAction(menuEvent(MenuAction.CREATE_SLOT))
                    finish()
                },
                onDismiss = ::finish,
            )
        }
    }

    override fun onDestroy() {
        MainActivity.slotListener = null
        super.onDestroy()
    }

    private fun postSlotAction(action: String) {
        try {
            sendSlotAction(action)
        } catch (e: Exception) {
            Log.e(TAG, "sendSlotAction failed", e)
        }
    }

    private external fun sendSlotAction(action: String)

    companion object {
        private const val TAG = "SlotActivity"

        fun createIntent(
            context: Context,
            slotsJson: String,
            orientation: Int = ActivityInfo.SCREEN_ORIENTATION_UNSPECIFIED,
        ): Intent =
            Intent(context, SlotActivity::class.java).apply {
                putExtra(EXTRA_SLOTS, slotsJson)
                putExtra(EXTRA_ORIENTATION, orientation)
            }
    }
}

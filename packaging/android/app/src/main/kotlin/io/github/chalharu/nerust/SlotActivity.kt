package io.github.chalharu.nerust

import android.content.Context
import android.content.Intent
import android.content.pm.ActivityInfo
import android.os.Bundle
import android.util.Log
import androidx.activity.ComponentActivity
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.platform.ComposeView
import androidx.compose.ui.platform.ViewCompositionStrategy
import androidx.lifecycle.setViewTreeLifecycleOwner
import androidx.lifecycle.setViewTreeViewModelStoreOwner
import androidx.savedstate.setViewTreeSavedStateRegistryOwner

private const val EXTRA_SLOTS = "nerust.extra.SLOTS"
private const val EXTRA_ORIENTATION = "nerust.extra.ORIENTATION"

/** Standalone save-slot manager. MainActivity passes one slot-list query
 * per open; select/create/delete ride the same menu lane as the drawer,
 * delete stays and refreshes from a native re-query. Being an Activity
 * (not a Dialog window) it looks, navigates, and backs out like the
 * settings screen instead of a centered popup. */
class SlotActivity : ComponentActivity() {
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
            val state = remember(raw) { SlotMenuState(raw, parseSlotList(raw)) }
            DisposableEffect(state) {
                onDispose { state.stop() }
            }
            SlotMenuScreen(
                rows = state.rows,
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
                    state.refreshAfter(::nativeQuerySlotList)
                },
                onCreate = {
                    postSlotAction(menuEvent(MenuAction.CREATE_SLOT))
                    finish()
                },
                onDismiss = ::finish,
            )
        }
    }

    private fun postSlotAction(action: String) {
        try {
            sendSlotAction(action)
        } catch (e: Exception) {
            Log.e(TAG, "sendSlotAction failed", e)
        }
    }

    private external fun nativeQuerySlotList(): String?

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

package io.github.chalharu.nerust

import android.app.Dialog
import android.content.Context
import android.graphics.drawable.ColorDrawable
import android.graphics.Color
import android.util.Log
import android.view.View
import android.view.ViewGroup
import android.view.WindowManager
import android.widget.FrameLayout
import androidx.compose.runtime.Composable
import androidx.compose.ui.platform.ComposeView
import androidx.compose.ui.platform.ViewCompositionStrategy

internal class ComposeOwnerFrameLayout(
    context: Context,
    private val installOwners: (View) -> Unit,
) : FrameLayout(context) {
    override fun onAttachedToWindow() {
        installOwners(this)
        installOwners(rootView)
        super.onAttachedToWindow()
    }
}

/** Full-screen Compose dialog host. Replacement acknowledges the old request;
 * request-scoped callbacks cannot release the replacement's native gate. */
class DialogManager(private val activity: MainActivity) {
    private var dialog: Dialog? = null
    private var rootView: View? = null
    private var composeView: View? = null
    var tag: String? = null
        private set
    private var dismissCallback: (() -> Unit)? = null
    var ownedByTest = false
        private set

    fun isShowing(tag: String): Boolean =
        this.tag == tag && dialog?.isShowing == true && rootView.isShownInWindowForTest()

    fun rootViewForTag(tag: String): View? = rootView.takeIf { this.tag == tag }

    fun dismissForTest() {
        dismiss(notifyDismiss = false)
    }

    /** Stop/destroy path: release the window like any other show failure. */
    fun dismissForStop() {
        dismiss(notifyDismiss = true)
    }

    fun resetForTest() {
        dismiss(notifyDismiss = !ownedByTest)
    }

    fun debugState(): String =
        "dialogTag=$tag, dialog=${dialog.debugDialogState()}, " +
            "dialogRoot=${rootView.debugViewState()}, dialogCompose=${composeView.debugViewState()}"

    fun show(
        dialogTag: String,
        contentDescription: String,
        presentation: String,
        onDismiss: () -> Unit,
        ownedByTest: Boolean = false,
        content: @Composable (dismiss: () -> Unit) -> Unit,
    ) {
        dismiss(notifyDismiss = !this.ownedByTest)

        lateinit var dialog: Dialog
        val root =
            ComposeOwnerFrameLayout(activity) { view ->
                activity.installComposeOwners(view)
            }.apply {
                tag = dialogTag
                setTag(R.id.nerust_dialog_content_probe, contentDescription)
                setTag(R.id.nerust_dialog_presentation_probe, presentation)
                layoutParams =
                    FrameLayout.LayoutParams(
                        ViewGroup.LayoutParams.MATCH_PARENT,
                        ViewGroup.LayoutParams.MATCH_PARENT,
                    )
            }
        val composeView =
            ComposeView(activity).apply {
                tag = "$dialogTag-compose"
                layoutParams =
                    FrameLayout.LayoutParams(
                        ViewGroup.LayoutParams.MATCH_PARENT,
                        ViewGroup.LayoutParams.MATCH_PARENT,
                    )
                setViewCompositionStrategy(ViewCompositionStrategy.DisposeOnDetachedFromWindow)
            }
        dialog =
            Dialog(activity).apply {
                window?.setBackgroundDrawable(ColorDrawable(Color.TRANSPARENT))
                setCancelable(true)
                setCanceledOnTouchOutside(true)
            }

        composeView.setContent {
            androidx.compose.material3.MaterialTheme {
                content { dialog.dismiss() }
            }
        }

        activity.installComposeOwners(root)
        activity.installComposeOwners(composeView)
        root.addView(composeView)
        dialog.setContentView(root)

        this.dialog = dialog
        rootView = root
        this.composeView = composeView
        tag = dialogTag
        dismissCallback = onDismiss
        this.ownedByTest = ownedByTest
        dialog.setOnDismissListener {
            // Android may already have queued this listener before replacement.
            // Never consume the replacement dialog's callback or references.
            if (this.dialog === dialog) {
                val callback = dismissCallback
                clearReferences()
                callback?.invoke()
            }
        }
        try {
            Log.i(TAG, "showComposeDialog: showing $dialogTag")
            dialog.show()
            dialog.window?.setLayout(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.MATCH_PARENT,
            )
            activity.lastDialogStateForTest = "showing $dialogTag"
            Log.i(TAG, "showComposeDialog: showing $dialogTag succeeded")
        } catch (error: WindowManager.BadTokenException) {
            dialog.setOnDismissListener(null)
            clearReferences()
            activity.lastDialogStateForTest = "show failed for $dialogTag: bad token"
            Log.w(TAG, "showComposeDialog: failed for $dialogTag with bad token", error)
            onDismiss()
        } catch (error: IllegalStateException) {
            dialog.setOnDismissListener(null)
            clearReferences()
            activity.lastDialogStateForTest = "show failed for $dialogTag: illegal state"
            Log.w(TAG, "showComposeDialog: failed for $dialogTag with illegal state", error)
            onDismiss()
        } catch (error: RuntimeException) {
            // Compose content threw on the main thread: release the
            // gate like any other show failure so Rust never wedges.
            dialog.setOnDismissListener(null)
            clearReferences()
            activity.lastDialogStateForTest = "show failed for $dialogTag: runtime"
            Log.w(TAG, "showComposeDialog: failed for $dialogTag with runtime error", error)
            onDismiss()
        }
    }

    fun dismiss(notifyDismiss: Boolean) {
        val dialog = dialog ?: return
        val callback = dismissCallback
        dialog.setOnDismissListener(null)
        clearReferences()
        dialog.dismiss()
        if (notifyDismiss) {
            callback?.invoke()
        }
    }

    private fun clearReferences() {
        dialog = null
        rootView = null
        composeView = null
        tag = null
        dismissCallback = null
        ownedByTest = false
    }

    private fun View?.isShownInWindowForTest(): Boolean =
        this != null && visibility == View.VISIBLE && isAttachedToWindow && windowToken != null

    private fun View?.debugViewState(): String =
        if (this == null) {
            "null"
        } else {
            "visibility=$visibility, attached=$isAttachedToWindow, token=${windowToken != null}, shown=$isShown"
        }

    private fun Dialog?.debugDialogState(): String = if (this == null) "null" else "showing=$isShowing"

    companion object {
        private const val TAG = "Nerust"
    }
}

package io.github.chalharu.nerust

import android.app.AlertDialog
import android.app.NativeActivity
import android.content.Intent
import android.util.Log
import android.widget.ArrayAdapter
import android.widget.ListView

class MainActivity : NativeActivity() {
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

    private external fun onFilePickerResult(uri: String?)

    private external fun onRomLibrarySelected(id: String?)

    private external fun onSettingsDialogResult(result: String?)

    companion object {
        private const val TAG = "Nerust"
        private const val ROM_PICKER_REQUEST_CODE = 0x4E45
        // Must match `android/library.rs::IMPORT_ACTION_ID`.
        private const val IMPORT_ACTION_ID = "__import__"
    }
}


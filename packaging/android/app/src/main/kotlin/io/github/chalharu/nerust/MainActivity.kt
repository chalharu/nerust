package io.github.chalharu.nerust

import android.app.NativeActivity
import android.content.Intent
import android.util.Log

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

    private external fun onFilePickerResult(uri: String?)

    companion object {
        private const val TAG = "Nerust"
        private const val ROM_PICKER_REQUEST_CODE = 0x4E45
    }
}

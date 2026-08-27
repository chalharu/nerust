package io.github.chalharu.nerust

import android.content.ContentProvider
import android.content.ContentValues
import android.database.Cursor
import android.database.MatrixCursor
import android.net.Uri
import android.os.ParcelFileDescriptor
import android.provider.OpenableColumns
import java.io.File

class TestRomProvider : ContentProvider() {
    override fun onCreate(): Boolean = true

    override fun getType(uri: Uri): String = "application/octet-stream"

    override fun query(
        uri: Uri,
        projection: Array<out String>?,
        selection: String?,
        selectionArgs: Array<out String>?,
        sortOrder: String?,
    ): Cursor {
        val columns = projection ?: arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE)
        return MatrixCursor(columns).apply {
            val values: Array<Any?> = columns.map { column ->
                when (column) {
                    OpenableColumns.DISPLAY_NAME -> "phase15-test.gbc"
                    OpenableColumns.SIZE -> TEST_ROM_SIZE
                    else -> null
                }
            }.toTypedArray()
            addRow(values)
        }
    }

    override fun openFile(uri: Uri, mode: String): ParcelFileDescriptor {
        val context = requireNotNull(context)
        val file = File(context.cacheDir, "phase15-test.gbc")
        file.writeBytes(minimalGbcRom())
        return ParcelFileDescriptor.open(file, ParcelFileDescriptor.MODE_READ_ONLY)
    }

    override fun insert(uri: Uri, values: ContentValues?): Uri? = null

    override fun delete(uri: Uri, selection: String?, selectionArgs: Array<out String>?): Int = 0

    override fun update(
        uri: Uri,
        values: ContentValues?,
        selection: String?,
        selectionArgs: Array<out String>?,
    ): Int = 0

    companion object {
        private const val TEST_ROM_SIZE = 0x8000
        val ROM_URI: Uri = Uri.parse("content://io.github.chalharu.nerust.testrom/phase15-test.gbc")

        private fun minimalGbcRom(): ByteArray {
            val rom = ByteArray(TEST_ROM_SIZE)
            val logo = byteArrayOf(
                0xCE.toByte(), 0xED.toByte(), 0x66, 0x66, 0xCC.toByte(), 0x0D, 0x00, 0x0B,
                0x03, 0x73, 0x00, 0x83.toByte(), 0x00, 0x0C, 0x00, 0x0D,
                0x00, 0x08, 0x11, 0x1F, 0x88.toByte(), 0x89.toByte(), 0x00, 0x0E,
                0xDC.toByte(), 0xCC.toByte(), 0x6E, 0xE6.toByte(), 0xDD.toByte(), 0xDD.toByte(), 0xD9.toByte(), 0x99.toByte(),
                0xBB.toByte(), 0xBB.toByte(), 0x67, 0x63, 0x6E, 0x0E, 0xEC.toByte(), 0xCC.toByte(),
                0xDD.toByte(), 0xDC.toByte(), 0x99.toByte(), 0x9F.toByte(), 0xBB.toByte(), 0xB9.toByte(), 0x33, 0x3E,
            )
            logo.copyInto(rom, destinationOffset = 0x0104)
            rom[0x0143] = 0x80.toByte()
            var checksum = 0
            for (index in 0x0134..0x014C) {
                checksum = (checksum - (rom[index].toInt() and 0xFF) - 1) and 0xFF
            }
            rom[0x014D] = checksum.toByte()
            return rom
        }
    }
}

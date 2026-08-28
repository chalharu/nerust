package io.github.chalharu.nerust;

import android.content.ContentProvider;
import android.content.ContentValues;
import android.content.Context;
import android.database.Cursor;
import android.database.MatrixCursor;
import android.net.Uri;
import android.os.ParcelFileDescriptor;
import android.provider.OpenableColumns;

import java.io.File;
import java.io.FileNotFoundException;
import java.io.FileOutputStream;
import java.io.IOException;

public final class TestRomProvider extends ContentProvider {
    private static final int TEST_ROM_SIZE = 0x8000;
    public static final Uri ROM_URI =
            Uri.parse("content://io.github.chalharu.nerust.testrom/phase15-test.gbc");

    @Override
    public boolean onCreate() {
        return true;
    }

    @Override
    public String getType(Uri uri) {
        return "application/octet-stream";
    }

    @Override
    public Cursor query(
            Uri uri,
            String[] projection,
            String selection,
            String[] selectionArgs,
            String sortOrder) {
        String[] columns = projection != null
                ? projection
                : new String[] {OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE};
        Object[] values = new Object[columns.length];
        for (int index = 0; index < columns.length; index++) {
            if (OpenableColumns.DISPLAY_NAME.equals(columns[index])) {
                values[index] = "phase15-test.gbc";
            } else if (OpenableColumns.SIZE.equals(columns[index])) {
                values[index] = TEST_ROM_SIZE;
            }
        }
        MatrixCursor cursor = new MatrixCursor(columns);
        cursor.addRow(values);
        return cursor;
    }

    @Override
    public ParcelFileDescriptor openFile(Uri uri, String mode) throws FileNotFoundException {
        Context context = getContext();
        if (context == null) {
            throw new FileNotFoundException("Test provider is not attached");
        }
        File file = new File(context.getCacheDir(), "phase15-test.gbc");
        try (FileOutputStream output = new FileOutputStream(file)) {
            output.write(minimalGbcRom());
        } catch (IOException error) {
            FileNotFoundException failure = new FileNotFoundException("Could not create test ROM");
            failure.initCause(error);
            throw failure;
        }
        return ParcelFileDescriptor.open(file, ParcelFileDescriptor.MODE_READ_ONLY);
    }

    @Override
    public Uri insert(Uri uri, ContentValues values) {
        return null;
    }

    @Override
    public int delete(Uri uri, String selection, String[] selectionArgs) {
        return 0;
    }

    @Override
    public int update(Uri uri, ContentValues values, String selection, String[] selectionArgs) {
        return 0;
    }

    private byte[] minimalGbcRom() {
        byte[] rom = new byte[TEST_ROM_SIZE];
        byte[] logo = {
            (byte) 0xCE, (byte) 0xED, 0x66, 0x66, (byte) 0xCC, 0x0D, 0x00, 0x0B,
            0x03, 0x73, 0x00, (byte) 0x83, 0x00, 0x0C, 0x00, 0x0D,
            0x00, 0x08, 0x11, 0x1F, (byte) 0x88, (byte) 0x89, 0x00, 0x0E,
            (byte) 0xDC, (byte) 0xCC, 0x6E, (byte) 0xE6, (byte) 0xDD, (byte) 0xDD, (byte) 0xD9, (byte) 0x99,
            (byte) 0xBB, (byte) 0xBB, 0x67, 0x63, 0x6E, 0x0E, (byte) 0xEC, (byte) 0xCC,
            (byte) 0xDD, (byte) 0xDC, (byte) 0x99, (byte) 0x9F, (byte) 0xBB, (byte) 0xB9, 0x33, 0x3E,
        };
        System.arraycopy(logo, 0, rom, 0x0104, logo.length);
        rom[0x0143] = (byte) 0x80;
        int checksum = 0;
        for (int index = 0x0134; index <= 0x014C; index++) {
            checksum = (checksum - (rom[index] & 0xFF) - 1) & 0xFF;
        }
        rom[0x014D] = (byte) checksum;
        return rom;
    }
}
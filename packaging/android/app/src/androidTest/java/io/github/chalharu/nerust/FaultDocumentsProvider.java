package io.github.chalharu.nerust;

import android.database.Cursor;
import android.database.MatrixCursor;
import android.net.Uri;
import android.os.CancellationSignal;
import android.os.ParcelFileDescriptor;
import android.provider.DocumentsContract;
import android.provider.DocumentsProvider;

import java.io.File;
import java.io.FileNotFoundException;
import java.io.FileOutputStream;
import java.io.IOException;
import java.util.concurrent.atomic.AtomicInteger;

/**
 * Fault-injecting stub DocumentsProvider for emulator abnormal-path tests
 * (NER-321/322/312). No physical device needed: the test drives outage,
 * corrupt, and oversized modes through static volatiles, and the provider
 * speaks the real DocumentsContract interface the production SafRepository
 * consumes. Authority is test-only and declared in the androidTest manifest.
 */
public final class FaultDocumentsProvider extends DocumentsProvider {
    public static final String AUTHORITY = "io.github.chalharu.nerust.faultdocs";
    public static final String ROOT_ID = "faultroot";
    public static final Uri TREE_URI =
            Uri.parse("content://" + AUTHORITY + "/tree/" + ROOT_ID);

    /** Remaining provider-wide transient failures; each query/open burns one. */
    public static final AtomicInteger outageFailures = new AtomicInteger(0);

    private static final String DOC_GOOD = DOC_GOOD;
    private static final String DOC_CORRUPT = DOC_CORRUPT;
    private static final String DOC_SLOT1_STATE = DOC_SLOT1_STATE;
    private static final String DOC_SUMMARY1 = DOC_SUMMARY1;
    private static final String DOC_SLOT2_STATE = DOC_SLOT2_STATE;
    private static final String DOC_SUMMARY2 = DOC_SUMMARY2;

    private static final byte[] GOOD_BYTES = "FAULT-PROVIDER-GOOD-BYTES".getBytes();
    private static final byte[] CORRUPT_BYTES = {
        (byte) 0xFF, (byte) 0xFE, 0x00, 0x01, 0x02, (byte) 0xFD, 0x03, 0x04,
    };
    private static final byte[] SMALL_SUMMARY = "{\"v\":1}".getBytes();
    /** 200 KiB: over the 64 KiB SAF_SUMMARY_BYTES cap. */
    public static final int HUGE_SUMMARY_SIZE = 200 * 1024;

    public static byte[] getGoodBytes() {
        return GOOD_BYTES.clone();
    }

    public static byte[] getCorruptBytes() {
        return CORRUPT_BYTES.clone();
    }

    private static final String[] CHILD_IDS = {
        DOC_GOOD, DOC_CORRUPT, DOC_SLOT1_STATE, DOC_SUMMARY1, DOC_SLOT2_STATE, DOC_SUMMARY2,
    };

    private static final String[] ROOT_PROJECTION = {
        DocumentsContract.Root.COLUMN_ROOT_ID,
        DocumentsContract.Root.COLUMN_FLAGS,
        DocumentsContract.Root.COLUMN_TITLE,
        DocumentsContract.Root.COLUMN_DOCUMENT_ID,
    };

    private static final String[] DOCUMENT_PROJECTION = {
        DocumentsContract.Document.COLUMN_DOCUMENT_ID,
        DocumentsContract.Document.COLUMN_DISPLAY_NAME,
        DocumentsContract.Document.COLUMN_MIME_TYPE,
        DocumentsContract.Document.COLUMN_SIZE,
        DocumentsContract.Document.COLUMN_LAST_MODIFIED,
        DocumentsContract.Document.COLUMN_FLAGS,
    };

    @Override
    public boolean onCreate() {
        return true;
    }

    private void burnOutage() {
        if (outageFailures.getAndDecrement() > 0) {
            throw new IllegalStateException("transient provider fault (injected)");
        }
    }

    @Override
    public Cursor queryRoots(String[] projection) {
        burnOutage();
        MatrixCursor cursor = new MatrixCursor(
                projection != null ? projection : ROOT_PROJECTION);
        MatrixCursor.RowBuilder row = cursor.newRow();
        for (String column : (projection != null ? projection : ROOT_PROJECTION)) {
            if (DocumentsContract.Root.COLUMN_ROOT_ID.equals(column)) {
                row.add(column, ROOT_ID);
            } else if (DocumentsContract.Root.COLUMN_FLAGS.equals(column)) {
                row.add(column, 0);
            } else if (DocumentsContract.Root.COLUMN_TITLE.equals(column)) {
                row.add(column, "fault");
            } else if (DocumentsContract.Root.COLUMN_DOCUMENT_ID.equals(column)) {
                row.add(column, ROOT_ID);
            } else {
                row.add(column, null);
            }
        }
        return cursor;
    }

    @Override
    public Cursor queryDocument(String documentId, String[] projection)
            throws FileNotFoundException {
        burnOutage();
        return cursorFor(documentId, projection);
    }

    @Override
    public Cursor queryChildDocuments(
            String parentDocumentId, String[] projection, String sortOrder)
            throws FileNotFoundException {
        burnOutage();
        if (!ROOT_ID.equals(parentDocumentId)) {
            throw new FileNotFoundException("unknown parent: " + parentDocumentId);
        }
        MatrixCursor cursor = new MatrixCursor(
                projection != null ? projection : DOCUMENT_PROJECTION);
        for (String childId : CHILD_IDS) {
            addRow(cursor, childId, projection);
        }
        return cursor;
    }

    private MatrixCursor cursorFor(String documentId, String[] projection)
            throws FileNotFoundException {
        if (ROOT_ID.equals(documentId)) {
            MatrixCursor cursor = new MatrixCursor(
                    projection != null ? projection : DOCUMENT_PROJECTION);
            MatrixCursor.RowBuilder row = cursor.newRow();
            for (String column : (projection != null ? projection : DOCUMENT_PROJECTION)) {
                row.add(column, rootValue(column));
            }
            return cursor;
        }
        for (String childId : CHILD_IDS) {
            if (childId.equals(documentId)) {
                MatrixCursor cursor = new MatrixCursor(
                        projection != null ? projection : DOCUMENT_PROJECTION);
                addRow(cursor, childId, projection);
                return cursor;
            }
        }
        throw new FileNotFoundException("unknown document: " + documentId);
    }

    private Object rootValue(String column) {
        if (DocumentsContract.Document.COLUMN_DOCUMENT_ID.equals(column)) {
            return ROOT_ID;
        }
        if (DocumentsContract.Document.COLUMN_DISPLAY_NAME.equals(column)) {
            return "fault";
        }
        if (DocumentsContract.Document.COLUMN_MIME_TYPE.equals(column)) {
            return DocumentsContract.Document.MIME_TYPE_DIR;
        }
        return null;
    }

    private void addRow(MatrixCursor cursor, String documentId, String[] projection) {
        MatrixCursor.RowBuilder row = cursor.newRow();
        for (String column : (projection != null ? projection : DOCUMENT_PROJECTION)) {
            row.add(column, columnValue(documentId, column));
        }
    }

    private Object columnValue(String documentId, String column) {
        if (DocumentsContract.Document.COLUMN_DOCUMENT_ID.equals(column)) {
            return documentId;
        }
        if (DocumentsContract.Document.COLUMN_DISPLAY_NAME.equals(column)) {
            return documentId;
        }
        if (DocumentsContract.Document.COLUMN_MIME_TYPE.equals(column)) {
            return "application/octet-stream";
        }
        if (DocumentsContract.Document.COLUMN_SIZE.equals(column)) {
            return (long) sizeOf(documentId);
        }
        if (DocumentsContract.Document.COLUMN_LAST_MODIFIED.equals(column)) {
            return 1_700_000_000_000L;
        }
        if (DocumentsContract.Document.COLUMN_FLAGS.equals(column)) {
            return 0;
        }
        return null;
    }

    private int sizeOf(String documentId) {
        if (DOC_GOOD.equals(documentId)) {
            return GOOD_BYTES.length;
        }
        if (DOC_CORRUPT.equals(documentId)) {
            return CORRUPT_BYTES.length;
        }
        if (DOC_SUMMARY1.equals(documentId)) {
            return SMALL_SUMMARY.length;
        }
        if (DOC_SUMMARY2.equals(documentId)) {
            return HUGE_SUMMARY_SIZE;
        }
        return 8;
    }

    private byte[] bytesOf(String documentId) {
        if (DOC_GOOD.equals(documentId)) {
            return GOOD_BYTES;
        }
        if (DOC_CORRUPT.equals(documentId)) {
            return CORRUPT_BYTES;
        }
        if (DOC_SUMMARY1.equals(documentId)) {
            return SMALL_SUMMARY;
        }
        if (DOC_SUMMARY2.equals(documentId)) {
            byte[] huge = new byte[HUGE_SUMMARY_SIZE];
            for (int i = 0; i < huge.length; i++) {
                huge[i] = (byte) (i & 0x7F);
            }
            return huge;
        }
        return new byte[] {0, 1, 2, 3, 4, 5, 6, 7};
    }

    @Override
    public ParcelFileDescriptor openDocument(
            String documentId, String mode, CancellationSignal signal)
            throws FileNotFoundException {
        burnOutage();
        boolean known = ROOT_ID.equals(documentId);
        for (String childId : CHILD_IDS) {
            known |= childId.equals(documentId);
        }
        if (!known || ROOT_ID.equals(documentId)) {
            throw new FileNotFoundException("cannot open: " + documentId);
        }
        if (getContext() == null) {
            throw new FileNotFoundException("provider is not attached");
        }
        File file = new File(getContext().getCacheDir(), "fault-" + documentId);
        try (FileOutputStream output = new FileOutputStream(file)) {
            output.write(bytesOf(documentId));
        } catch (IOException error) {
            FileNotFoundException failure =
                    new FileNotFoundException("could not stage fault document");
            failure.initCause(error);
            throw failure;
        }
        return ParcelFileDescriptor.open(file, ParcelFileDescriptor.MODE_READ_ONLY);
    }
}

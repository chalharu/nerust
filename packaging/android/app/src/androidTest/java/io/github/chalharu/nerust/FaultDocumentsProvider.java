package io.github.chalharu.nerust;

import android.content.ContentProvider;
import android.content.ContentValues;
import android.database.Cursor;
import android.database.MatrixCursor;
import android.net.Uri;
import android.os.Bundle;
import android.os.ParcelFileDescriptor;
import android.provider.DocumentsContract;

import java.io.File;
import java.io.FileNotFoundException;
import java.io.FileOutputStream;
import java.io.IOException;
import java.util.List;
import java.util.concurrent.atomic.AtomicInteger;

/** Fault-injecting stub provider for emulator abnormal-path tests. */
public final class FaultDocumentsProvider extends ContentProvider {
    public static final String AUTHORITY = "io.github.chalharu.nerust.faultdocs";
    public static final String ROOT_ID = "faultroot";
    public static final Uri TREE_URI =
            Uri.parse("content://" + AUTHORITY + "/tree/" + ROOT_ID);
    public static final Uri COMMAND_URI =
            Uri.parse("content://" + AUTHORITY + "/command");
    /** {@link #call} method selecting the remaining outage failure count. */
    public static final String SET_OUTAGE_METHOD = "setOutageFailures";

    /** Remaining provider-wide transient failures; each query/open burns one. */
    private static final AtomicInteger outageFailures = new AtomicInteger(0);

    private static final String DOC_GOOD = "good.state";
    private static final String DOC_CORRUPT = "corrupt.state";
    private static final String DOC_SLOT1_STATE = "slot1.state";
    private static final String DOC_SUMMARY1 = "slot1.summary";
    private static final String DOC_SLOT2_STATE = "slot2.state";
    private static final String DOC_SUMMARY2 = "slot2.summary";

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

    /** Tree-descendant document id: matches real provider id shapes. */
    private static String docId(String leaf) {
        return ROOT_ID + "/" + leaf;
    }

    /** Leaf display name for a tree-descendant document id. */
    private static String leafOf(String documentId) {
        String prefix = ROOT_ID + "/";
        if (documentId != null && documentId.startsWith(prefix)) {
            return documentId.substring(prefix.length());
        }
        return documentId;
    }

    private static boolean isTreeShape(List<String> segments) {
        return segments.size() >= 4
                && "tree".equals(segments.get(0))
                && "document".equals(segments.get(2));
    }

    private static boolean isChildrenUri(List<String> segments) {
        return segments.size() == 5 && isTreeShape(segments) && "children".equals(segments.get(4));
    }

    private static boolean isDocumentUri(List<String> segments) {
        return segments.size() == 4 && isTreeShape(segments);
    }

    @Override
    public Cursor query(
            Uri uri, String[] projection, String selection, String[] selectionArgs, String sortOrder) {
        burnOutage();
        List<String> segments = uri.getPathSegments();
        if (isChildrenUri(segments)) {
            String parent = segments.get(3);
            if (!ROOT_ID.equals(parent)) {
                throw new IllegalArgumentException("unknown parent: " + parent);
            }
            MatrixCursor cursor = new MatrixCursor(
                    projection != null ? projection : DOCUMENT_PROJECTION);
            for (String childId : CHILD_IDS) {
                addRow(cursor, docId(childId), projection);
            }
            return cursor;
        }
        if (isDocumentUri(segments)) {
            return cursorFor(segments.get(3), projection);
        }
        throw new IllegalArgumentException("unsupported uri: " + uri);
    }

    @Override
    public Bundle call(String method, String arg, Bundle extras) {
        // Control plane: never burns outage, so an active outage cannot
        // block recovery. Test-only; unsupported methods fail loudly.
        if (SET_OUTAGE_METHOD.equals(method)) {
            try {
                outageFailures.set(Integer.parseInt(arg));
            } catch (NumberFormatException error) {
                throw new IllegalArgumentException("bad outage count: " + arg);
            }
            return null;
        }
        throw new IllegalArgumentException("unsupported call: " + method);
    }

    private MatrixCursor cursorFor(String documentId, String[] projection) {
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
            if (docId(childId).equals(documentId)) {
                MatrixCursor cursor = new MatrixCursor(
                        projection != null ? projection : DOCUMENT_PROJECTION);
                addRow(cursor, documentId, projection);
                return cursor;
            }
        }
        throw new IllegalArgumentException("unknown document: " + documentId);
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
            return leafOf(documentId);
        }
        if (DocumentsContract.Document.COLUMN_MIME_TYPE.equals(column)) {
            return "application/octet-stream";
        }
        if (DocumentsContract.Document.COLUMN_SIZE.equals(column)) {
            return (long) sizeOf(leafOf(documentId));
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
    public ParcelFileDescriptor openFile(Uri uri, String mode) throws FileNotFoundException {
        burnOutage();
        List<String> segments = uri.getPathSegments();
        if (!isDocumentUri(segments)) {
            throw new FileNotFoundException("cannot open: " + uri);
        }
        String leaf = leafOf(segments.get(3));
        boolean known = false;
        for (String childId : CHILD_IDS) {
            known |= childId.equals(leaf);
        }
        if (!known) {
            throw new FileNotFoundException("cannot open: " + uri);
        }
        if (getContext() == null) {
            throw new FileNotFoundException("provider is not attached");
        }
        File file = new File(getContext().getCacheDir(), "fault-" + leaf);
        try (FileOutputStream output = new FileOutputStream(file)) {
            output.write(bytesOf(leaf));
        } catch (IOException error) {
            FileNotFoundException failure =
                    new FileNotFoundException("could not stage fault document");
            failure.initCause(error);
            throw failure;
        }
        return ParcelFileDescriptor.open(file, ParcelFileDescriptor.MODE_READ_ONLY);
    }

    @Override
    public String getType(Uri uri) {
        return "application/octet-stream";
    }

    @Override
    public Uri insert(Uri uri, ContentValues values) {
        throw new UnsupportedOperationException("read-only stub");
    }

    @Override
    public int delete(Uri uri, String selection, String[] selectionArgs) {
        throw new UnsupportedOperationException("read-only stub");
    }

    @Override
    public int update(Uri uri, ContentValues values, String selection, String[] selectionArgs) {
        throw new UnsupportedOperationException("read-only stub");
    }
}

package jp.chalharu.nerust;

public final class RustBridge {
    static {
        // JNI のライブラリ (モジュール) をロードします。
        System.loadLibrary("librust");
    }

    static native void onSurfaceCreated();

    static native void onSurfaceChanged(int width, int height);

    static native void onDrawFrame();
}

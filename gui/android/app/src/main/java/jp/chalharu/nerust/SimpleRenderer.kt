package jp.chalharu.nerust

import android.opengl.GLSurfaceView
import javax.microedition.khronos.opengles.GL10
import android.opengl.ETC1.getHeight
import android.opengl.ETC1.getWidth
import android.R
import android.content.Context
import android.graphics.BitmapFactory
import android.graphics.Bitmap
import javax.microedition.khronos.egl.EGLConfig


class SimpleRenderer(context: Context): GLSurfaceView.Renderer {
    private var _context: Context

    init {
        _context = context
    }

    override fun onSurfaceCreated(_gl: GL10, _config: EGLConfig) {
        RustBridge.onSurfaceCreated()
    }

    override fun onSurfaceChanged(_gl: GL10, width: Int, height: Int) {
        RustBridge.onSurfaceChanged(width, height)
    }

    override fun onDrawFrame(_gl: GL10) {
        RustBridge.onDrawFrame()
    }
}
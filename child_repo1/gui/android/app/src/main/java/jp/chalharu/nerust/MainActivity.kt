package jp.chalharu.nerust

import android.opengl.GLSurfaceView
import androidx.appcompat.app.AppCompatActivity
import android.os.Bundle

class MainActivity : AppCompatActivity() {

    private val _glSurfaceView: GLSurfaceView = GLSurfaceView(this)

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_main)

        _glSurfaceView.setEGLContextClientVersion(2)	// OpenGL ES 2.0 を使用するように構成します。
        _glSurfaceView.setRenderer(SimpleRenderer(getApplicationContext()))
        setContentView(_glSurfaceView)
    }
}

use jni::EnvUnowned;
use jni::objects::JClass;
use jni::sys::jint;

#[unsafe(no_mangle)]
#[allow(non_snake_case)]
pub extern "system" fn Java_jp_chalharu_nerust_SimpleRenderer_onSurfaceCreated(
    _env: EnvUnowned<'_>,
    _class: JClass<'_>,
) {
}

#[unsafe(no_mangle)]
#[allow(non_snake_case)]
pub extern "system" fn Java_jp_chalharu_nerust_SimpleRenderer_onSurfaceChanged(
    _env: EnvUnowned<'_>,
    _class: JClass<'_>,
    _width: jint,
    _height: jint,
) {
    // glViewport(0, 0, width, height);
    // checkGlError("glViewport");
}

#[unsafe(no_mangle)]
#[allow(non_snake_case)]
pub extern "system" fn Java_jp_chalharu_nerust_SimpleRenderer_onDrawFrame(
    _env: EnvUnowned<'_>,
    _class: JClass<'_>,
) {
}

use jni::objects::{JClass, JString};
use jni::sys::{jint, jstring};
use jni::JNIEnv;

#[no_mangle]
#[allow(non_snake_case)]
pub extern "system" fn Java_jp_chalharu_nerust_SimpleRenderer_onSurfaceCreated(
    env: JNIEnv,
    class: JClass,
) {
}

#[no_mangle]
#[allow(non_snake_case)]
pub extern "system" fn Java_jp_chalharu_nerust_SimpleRenderer_onSurfaceChanged(
    env: JNIEnv,
    class: JClass,
    width: jint,
    height: jint,
) {
    // glViewport(0, 0, width, height);
    // checkGlError("glViewport");
}

#[no_mangle]
#[allow(non_snake_case)]
pub extern "system" fn Java_jp_chalharu_nerust_SimpleRenderer_onDrawFrame(
    env: JNIEnv,
    class: JClass,
) {
}

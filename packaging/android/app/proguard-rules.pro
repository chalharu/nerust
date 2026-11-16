# Keep JNI bridge: native methods are resolved by name at runtime.
-keepclasseswithmembernames class * {
    native <methods>;
}
-keep class io.github.chalharu.nerust.MainActivity { *; }
-keep class io.github.chalharu.nerust.SettingsActivity { *; }
-keep class io.github.chalharu.nerust.SlotActivity { *; }

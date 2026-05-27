plugins {
    id("com.android.application")
}

fun envOrNull(name: String): String? = System.getenv(name)?.takeUnless(String::isBlank)

val versionNameOverride = System.getenv("ANDROID_VERSION_NAME") ?: "0.1.0"
val versionCodeOverride = System.getenv("ANDROID_VERSION_CODE")?.toIntOrNull() ?: 1
val keystoreFilePath = envOrNull("ANDROID_KEY_STORE_PATH")
val keystoreAlias = envOrNull("ANDROID_KEY_ALIAS")
val keystoreStorePassword = envOrNull("ANDROID_KEY_STORE_PASSWORD")
val keystoreKeyPassword = envOrNull("ANDROID_KEY_PASSWORD")

android {
    namespace = "io.github.chalharu.nerust"
    compileSdk = 35

    defaultConfig {
        applicationId = "io.github.chalharu.nerust"
        minSdk = 26
        targetSdk = 35
        versionCode = versionCodeOverride
        versionName = versionNameOverride
        ndk {
            abiFilters += "arm64-v8a"
        }
    }

    signingConfigs {
        if (
            keystoreFilePath != null &&
                keystoreAlias != null &&
                keystoreStorePassword != null &&
                keystoreKeyPassword != null
        ) {
            create("release") {
                storeFile = file(keystoreFilePath)
                storePassword = keystoreStorePassword
                keyAlias = keystoreAlias
                keyPassword = keystoreKeyPassword
            }
        }
    }

    buildTypes {
        getByName("debug") {
            isMinifyEnabled = false
        }
        getByName("release") {
            isMinifyEnabled = false
            signingConfig = signingConfigs.findByName("release") ?: signingConfigs.getByName("debug")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    sourceSets {
        getByName("main") {
            jniLibs.srcDir("src/main/jniLibs")
        }
    }
}

kotlin {
    compilerOptions {
        jvmTarget = org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17
    }
}

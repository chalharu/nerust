plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

val versionNameOverride = System.getenv("ANDROID_VERSION_NAME") ?: "0.1.0"
val versionCodeOverride = System.getenv("ANDROID_VERSION_CODE")?.toIntOrNull() ?: 1
val keystoreFilePath = System.getenv("ANDROID_KEY_STORE_PATH")
val keystoreAlias = System.getenv("ANDROID_KEY_ALIAS")
val keystoreStorePassword = System.getenv("ANDROID_KEY_STORE_PASSWORD")
val keystoreKeyPassword = System.getenv("ANDROID_KEY_PASSWORD")

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

    kotlinOptions {
        jvmTarget = "17"
    }

    sourceSets {
        getByName("main") {
            jniLibs.srcDirs("src/main/jniLibs")
        }
    }
}

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.plugin.compose")
}

fun envOrNull(name: String): String? = System.getenv(name)?.takeUnless(String::isBlank)

val versionNameOverride = System.getenv("ANDROID_VERSION_NAME") ?: "0.1.0"
val versionCodeOverride = System.getenv("ANDROID_VERSION_CODE")?.toIntOrNull() ?: 1
val abiFiltersOverride =
    envOrNull("ANDROID_ABI_FILTERS")
        ?.split(",")
        ?.map(String::trim)
        ?.filter(String::isNotEmpty)
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
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        versionCode = versionCodeOverride
        versionName = versionNameOverride
        ndk {
            abiFilters += abiFiltersOverride ?: listOf("arm64-v8a")
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

    buildFeatures {
        compose = true
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

dependencies {
    val composeBom = platform("androidx.compose:compose-bom:2026.05.01")
    val lifecycleVersion = "2.10.0"

    implementation(composeBom)
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.ui:ui")
    implementation("androidx.lifecycle:lifecycle-runtime:$lifecycleVersion")
    implementation("androidx.lifecycle:lifecycle-viewmodel:$lifecycleVersion")
    implementation("androidx.savedstate:savedstate:1.5.0")

    androidTestImplementation("androidx.test:core:1.7.0")
    androidTestImplementation("androidx.test:runner:1.7.0")
    androidTestImplementation("androidx.test:rules:1.7.0")
    androidTestImplementation("androidx.test.ext:junit:1.3.0")
    androidTestImplementation("androidx.test.uiautomator:uiautomator:2.4.0-beta02")
    androidTestImplementation("junit:junit:4.13.2")
}

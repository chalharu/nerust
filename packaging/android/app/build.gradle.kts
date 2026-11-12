plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.plugin.compose")
}

dependencyLocking {
    lockAllConfigurations()
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
    compileSdk = 37

    defaultConfig {
        applicationId = "io.github.chalharu.nerust"
        minSdk = 28
        targetSdk = 37
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
        buildConfig = true
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
    val composeBomVersion = "2026.09.00"
    val material3Version = "1.5.0-rc01"
    val composeBom = platform("androidx.compose:compose-bom:$composeBomVersion")
    val documentFileVersion = "1.1.0"
    val junitVersion = "4.13.2"
    val lifecycleVersion = "2.11.0"
    val savedStateVersion = "1.5.0"
    val androidxTestCoreVersion = "1.7.0"
    val androidxTestRunnerVersion = androidxTestCoreVersion
    val androidxTestRulesVersion = androidxTestCoreVersion
    val androidxTestExtVersion = "1.3.0"
    // ComponentActivity for the standalone settings screen (lifecycle +
    // back dispatcher owners without hand-rolled registries). Pinned to
    // the version the BOM already resolves so the lockfile stands.
    val activityComposeVersion = "1.8.2"

    implementation(composeBom)
    implementation("androidx.activity:activity-compose:$activityComposeVersion")
    // Material 3 Expressive (settings screen redesign): pin the 1.5.0 line
    // where MaterialExpressiveTheme/MotionScheme.expressive() live. Lift to
    // stable 1.5.x once the BOM manages it.
    implementation("androidx.compose.material3:material3:$material3Version")
    // Icons are Material Symbols bundled as vectors (settings + drawer):
    // the deprecated icons library and -extended (thousands of unused
    // icons, minify off) stay out, so no icon dependency ships at all.
    implementation("androidx.compose.ui:ui")
    implementation("androidx.documentfile:documentfile:$documentFileVersion")
    implementation("androidx.lifecycle:lifecycle-runtime:$lifecycleVersion")
    implementation("androidx.lifecycle:lifecycle-viewmodel:$lifecycleVersion")
    implementation("androidx.savedstate:savedstate:$savedStateVersion")

    testImplementation("junit:junit:$junitVersion")

    androidTestImplementation("androidx.test:core:$androidxTestCoreVersion")
    androidTestImplementation("androidx.test:runner:$androidxTestRunnerVersion")
    androidTestImplementation("androidx.test:rules:$androidxTestRulesVersion")
    androidTestImplementation("androidx.test.ext:junit:$androidxTestExtVersion")
    androidTestImplementation("junit:junit:$junitVersion")
}

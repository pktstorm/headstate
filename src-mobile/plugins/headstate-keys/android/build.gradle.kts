// The Android side of tauri-plugin-headstate-keys. Wired into the app's
// Gradle build by the Tauri CLI (build.rs publishes this directory as
// the plugin's `android_library_path`); it is not built by anything in
// the Makefile, and there is no Android SDK on the machine that wrote
// it -- see the note in the PR that added it.
plugins {
    id("com.android.library")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "com.pktstorm.headstate.keys"
    // The same level as the Tauri Android API module this depends on.
    // The ML-DSA constants are API 37; the plugin spells their documented
    // string values instead so this module compiles against the SDK the
    // Tauri template already needs.
    compileSdk = 36

    defaultConfig {
        // Tauri's floor. Below API 30 a key cannot be gated on
        // "biometric OR device credential", so those devices get a
        // biometric-only step-up key (HeadstateKeysPlugin.kt).
        minSdk = 24
        consumerProguardFiles("consumer-rules.pro")
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_1_8
        targetCompatibility = JavaVersion.VERSION_1_8
    }
    kotlinOptions {
        jvmTarget = "1.8"
    }
}

dependencies {
    implementation("androidx.core:core-ktx:1.9.0")
    implementation("androidx.appcompat:appcompat:1.6.0")
    // The system prompt that authorises a Keystore operation. 1.1.0 is
    // the stable line; it hosts the prompt in a FragmentActivity, which
    // Tauri's activity (an AppCompatActivity) is.
    implementation("androidx.biometric:biometric:1.1.0")
    implementation(project(":tauri-android"))
}

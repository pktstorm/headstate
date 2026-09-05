// The Android side of tauri-plugin-headstate-refresh. Wired into the
// app's Gradle build by the Tauri CLI (build.rs publishes this directory
// as the plugin's `android_library_path`); it is not built by anything
// in the Makefile, and there is no Android SDK on the machine that wrote
// it -- see the note in the PR that added it.
plugins {
    id("com.android.library")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "com.pktstorm.headstate.refresh"
    // The same level as the Tauri Android API module this depends on.
    compileSdk = 36

    defaultConfig {
        // Tauri's floor.
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
    // The periodic background window (RefreshWorker.kt). 2.9.x is the
    // stable line that still builds against Java 8 bytecode, which the
    // Tauri template's modules target.
    implementation("androidx.work:work-runtime-ktx:2.9.1")
    implementation(project(":tauri-android"))
}

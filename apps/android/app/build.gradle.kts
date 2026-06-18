import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {
    id("com.android.application")
    // AGP 9 ships built-in Kotlin; only the Compose compiler plugin is applied explicitly.
    id("org.jetbrains.kotlin.plugin.compose")
}

android {
    namespace = "io.liberalize.dogtag"
    // Use an installed platform (android-36 is present in the SDK).
    compileSdk = 36

    defaultConfig {
        applicationId = "io.liberalize.dogtag"
        minSdk = 26
        targetSdk = 34
        versionCode = 1
        versionName = "0.1-phase6"
        // The connected Xiaomi (Redmi 23129RN51X) is a 32-bit armeabi-v7a device; arm64 is also
        // built for newer devices. Both .so variants are bundled in jniLibs.
        ndk {
            abiFilters += "armeabi-v7a"
            abiFilters += "arm64-v8a"
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
        }
        debug {
            isMinifyEnabled = false
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    buildFeatures {
        compose = true
    }
}

kotlin {
    compilerOptions {
        jvmTarget.set(JvmTarget.JVM_17)
    }
}

dependencies {
    implementation("androidx.core:core-ktx:1.13.1")
    implementation("androidx.activity:activity-compose:1.9.3")
    implementation(platform("androidx.compose:compose-bom:2024.10.01"))
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.ui:ui-tooling-preview")
    // UniFFI Kotlin bindings require JNA (the @aar variant ships the native JNI libs for Android).
    implementation("net.java.dev.jna:jna:5.14.0@aar")
}

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

    // The ~65 MB Groth16 proving key is bundled as a raw asset and read as a file path by the
    // on-device prover. It MUST NOT be compressed, or the AssetManager cannot return a stable
    // on-disk path / the copy-out would have to inflate it.
    androidResources {
        noCompress += "zkey"
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
    implementation("androidx.compose.material:material-icons-extended")
    implementation("androidx.compose.ui:ui-tooling-preview")
    implementation("androidx.navigation:navigation-compose:2.8.4")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.8.7")

    // Persisted theme selection / wallet metadata.
    implementation("androidx.datastore:datastore-preferences:1.1.1")

    // CameraX + ML Kit barcode scanning (QR).
    implementation("androidx.camera:camera-core:1.4.1")
    implementation("androidx.camera:camera-camera2:1.4.1")
    implementation("androidx.camera:camera-lifecycle:1.4.1")
    implementation("androidx.camera:camera-view:1.4.1")
    implementation("com.google.mlkit:barcode-scanning:17.3.0")

    // Embedded wallet: Android Keystore-gated secret + BIP-39/secp256k1 + biometric prompt.
    implementation("androidx.security:security-crypto:1.1.0-alpha06")
    implementation("androidx.biometric:biometric:1.1.0")
    implementation("org.bouncycastle:bcprov-jdk18on:1.78.1")

    // UniFFI Kotlin bindings require JNA (the @aar variant ships the native JNI libs for Android).
    implementation("net.java.dev.jna:jna:5.14.0@aar")
}

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
        // `BuildConfig.DEBUG` gates the debug-only on-device ZK self-test (Profile screen) that the
        // Maestro mobile e2e drives — it must never appear in a release build.
        buildConfig = true
    }

    // The ~65 MB Groth16 proving key and the ~3 MB witness graph are bundled as raw assets and read
    // as file paths by the on-device prover. They MUST NOT be compressed, or the AssetManager cannot
    // return a stable on-disk path / the copy-out would have to inflate them.
    androidResources {
        noCompress += "zkey"
        noCompress += "graph"
    }

    // Robolectric-backed unit tests (QrPayloadTest) need the Android resource/asset table on the
    // unit-test classpath; without this the runtime cannot initialise and every such test errors out.
    testOptions {
        unitTests.isIncludeAndroidResources = true
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

    // QR GENERATION (pure-Java) for the travel-receipt public-status QR. ML Kit above only SCANS;
    // the holder receipt must DRAW a PII-free /r/:receiptId QR, so we encode with zxing-core.
    implementation("com.google.zxing:core:3.5.3")

    // JVM unit tests (pure-JVM, no Android runtime): pins the on-chain `isValid` selector derivation
    // (RoaxRpcSelectorTest) so it can never drift back to a reverting value. Run: `./gradlew test`.
    testImplementation("junit:junit:4.13.2")

    // Robolectric supplies the Android runtime classes that a few units genuinely cannot be tested
    // without: `QrPayload.parse` is built on `android.net.Uri`, and QR content is fully attacker-
    // controlled, so its edge cases (duplicate query keys, malformed input) must be covered by a test
    // that exercises the REAL Uri parser rather than a hand-rolled stand-in. Still `./gradlew test`.
    testImplementation("org.robolectric:robolectric:4.12.2")

    // The DESKTOP JNA jar, for JVM unit tests that call the Rust core over UniFFI
    // (`ProfileTreeParityTest`). The `@aar` variant above ships `libjnidispatch.so` for Android ABIs
    // only, so on a host JVM `Native.load` has no dispatcher to bind with; this jar carries the
    // macOS/Linux `libjnidispatch` and is test-scope only, so the shipped APK is unchanged.
    testImplementation("net.java.dev.jna:jna:5.14.0")
}

// ---- host-native Rust core, for the JVM parity test -------------------------------------------
//
// `apps/android/app/src/main/jniLibs/**/libdogtag_standard.so` is built by cargo-ndk for ANDROID
// ABIs and is gitignored (docs/MOBILE_BUILD.md), so it can never be loaded by a test running on the
// developer's own JVM. The cross-platform `R` parity test needs the REAL Rust implementation - a
// stubbed one would assert Kotlin against Kotlin and prove nothing - so we build the same crate for
// the HOST here and point JNA at it.
//
// Deliberately a hard dependency of `test`, not a soft skip or an `onlyIf`: this test is the only
// thing standing between an Android wrapper bug and a tag whose `R` the circuit cannot prove, and a
// self-skipping parity test reports green in exactly the situation it exists to catch. `cargo` is
// already a documented prerequisite of the Android build. A COLD build is slow - `--features prover`
// pulls the whole ark-groth16/ark-circom tree - which is exactly why the inputs/outputs below matter:
// without them Gradle can never mark the task up to date and every `test` invocation re-shells to
// cargo. Warm and unchanged, it is now a no-op.
val workspaceRoot = rootDir.parentFile.parentFile
val hostRustCoreDir = File(workspaceRoot, "target/debug")

val buildHostRustCore by tasks.registering(Exec::class) {
    group = "verification"
    description = "Builds dogtag-standard-rs for the host so JVM unit tests can load it over JNA."
    workingDir = workspaceRoot
    inputs.dir(File(workspaceRoot, "crates/dogtag-standard-rs/src")).withPathSensitivity(PathSensitivity.RELATIVE)
    inputs.file(File(workspaceRoot, "crates/dogtag-standard-rs/build.rs"))
    inputs.file(File(workspaceRoot, "crates/dogtag-standard-rs/Cargo.toml"))
    inputs.file(File(workspaceRoot, "Cargo.lock"))
    // The host artifact is `.dylib` on macOS and `.so` on Linux, so match on the stem rather than
    // hardcoding either extension.
    outputs.files(fileTree(hostRustCoreDir) { include("libdogtag_standard.*") })
    // `--features prover` is REQUIRED, not an optimisation: the checked-in Kotlin bindings were
    // generated from the prover-enabled crate, and UniFFI verifies EVERY exported function's
    // checksum when the library is first loaded. A default-feature build omits `prove_consent`, so
    // that load-time sweep dies with `UnsatisfiedLinkError: … checksum_func_prove_consent` before a
    // single profile-tree call runs. Matches the iOS workflow's xcframework build.
    commandLine("cargo", "build", "-p", "dogtag-standard-rs", "--features", "prover", "--lib")
}

tasks.withType<Test>().configureEach {
    dependsOn(buildHostRustCore)
    // Where `Native.load("dogtag_standard")` looks for lib{dogtag_standard}.{dylib,so}.
    systemProperty("jna.library.path", hostRustCoreDir.absolutePath)
    // Tests read shared cross-language fixtures from the repo. The unit-test working directory is
    // the MODULE dir (`apps/android/app`), which is an AGP detail no test should encode as `../../..`.
    systemProperty("dogtag.repoRoot", workspaceRoot.absolutePath)
}

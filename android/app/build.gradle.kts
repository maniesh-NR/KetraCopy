plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
}

android {
    namespace = "io.ketracopy"
    compileSdk = 35

    defaultConfig {
        applicationId = "io.ketracopy"
        minSdk = 26
        targetSdk = 35
        versionCode = 1
        versionName = "1.0"
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        vectorDrawables { useSupportLibrary = true }
    }

    // The Rust cdylib (ketra-ffi) is cross-compiled for the three main
    // Android ABIs by the `buildRustLibs` Gradle task (see below), which
    // invokes `cargo ndk` against the workspace at ../crates/ketra-ffi.
    // The resulting libketraffi.so files land in jniLibs/<abi>/ and are
    // packaged into the APK automatically.
    sourceSets["main"].jniLibs.srcDirs("src/main/jniLibs")

    buildTypes {
        release {
            isMinifyEnabled = false
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions {
        jvmTarget = "17"
    }
    buildFeatures {
        compose = true
    }
    packaging {
        resources {
            excludes += "/META-INF/{AL2.0,LGPL2.1}"
        }
    }
}

dependencies {
    // AndroidX + Compose
    implementation("androidx.core:core-ktx:1.15.0")
    implementation("androidx.lifecycle:lifecycle-runtime-ktx:2.8.7")
    implementation("androidx.lifecycle:lifecycle-viewmodel-compose:2.8.7")
    implementation("androidx.activity:activity-compose:1.9.3")
    implementation(platform("androidx.compose:compose-bom:2024.12.01"))
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.ui:ui-graphics")
    implementation("androidx.compose.ui:ui-tooling-preview")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.material:material-icons-extended")
    implementation("androidx.navigation:navigation-compose:2.8.5")
    implementation("androidx.documentfile:documentfile:1.0.1")

    // UniFFI Kotlin runtime — the generated Ketra.kt calls into this.
    implementation("net.rustlings:uniffi-android:0.28.3")

    // WebDAV provider (simple OkHttp + XML parsing).
    implementation("com.squareup.okhttp3:okhttp:4.12.0")

    testImplementation("junit:junit:4.13.2")
    androidTestImplementation("androidx.test.ext:junit:1.2.1")
    androidTestImplementation("androidx.test.espresso:espresso-core:3.6.1")
    androidTestImplementation(platform("androidx.compose:compose-bom:2024.12.01"))
    androidTestImplementation("androidx.compose.ui:ui-test-junit4")
    debugImplementation("androidx.compose.ui:ui-tooling")
    debugImplementation("androidx.compose.ui:ui-test-manifest")
}

// ============================================================================
// Rust cross-compilation via cargo-ndk
// ============================================================================
//
// We don't depend on the cargo-ndk Gradle plugin (it pulls a hard dependency
// on a specific NDK version). Instead we shell out to `cargo ndk` directly
// from a task that runs before assembleDebug. The GitHub Actions workflow
// installs cargo-ndk + the NDK + the android targets first.
//
// To build locally you need:
//   rustup target add arm64-v8a armeabi-v7a x86_64
//   cargo install cargo-ndk
//   ANDROID_NDK_HOME set (or ndkVersion set below)
//
// Skipping this task is fine for Compose UI iteration — the app will launch
// but crash on the first UniFFI call until the .so is present.

val rustAbis = listOf("arm64-v8a", "armeabi-v7a", "x86_64")
val rustTargets = mapOf(
    "arm64-v8a" to "aarch64-linux-android",
    "armeabi-v7a" to "armv7-linux-androideabi",
    "x86_64" to "x86_64-linux-android",
)
val rustWorkspaceDir = file("../").canonicalFile  // the ketracopy workspace root

val buildRustLibs = tasks.register<Exec>("buildRustLibs") {
    group = "rust"
    description = "Cross-compile libketraffi.so for all Android ABIs via cargo-ndk."
    workingDir = rustWorkspaceDir
    val outDir = file("src/main/jniLibs")
    commandLine = listOf(
        "cargo", "ndk",
        "-t", rustAbis[0],
        "-t", rustAbis[1],
        "-t", rustAbis[2],
        "build", "--release", "-p", "ketra-ffi",
        "--target-dir", outDir.canonicalPath,
    )
    // Move the artifacts from <outDir>/<triple>/release/libketraffi.so to
    // jniLibs/<abi>/ via a doLast step.
    doLast {
        rustAbis.forEach { abi ->
            val triple = rustTargets[abi]!!
            val src = file("src/main/jniLibs/$triple/release/libketraffi.so")
            val dstDir = file("src/main/jniLibs/$abi")
            dstDir.mkdirs()
            if (src.exists()) {
                src.copyTo(file("${dstDir}/libketraffi.so"), overwrite = true)
            } else {
                logger.warn("libketraffi.so for $abi not found at ${src.canonicalPath}; the app will not load the Rust engine.")
            }
        }
    }
    // Re-run only if the Rust sources changed.
    inputs.dir(file("$rustWorkspaceDir/crates/ketra-ffi/src"))
    inputs.dir(file("$rustWorkspaceDir/crates/ketra-net/src"))
    inputs.dir(file("$rustWorkspaceDir/crates/ketra-core/src"))
    inputs.file(file("$rustWorkspaceDir/Cargo.toml"))
    outputs.dir(outDir)
}

// Generate the UniFFI Kotlin bindings before compile. In UniFFI 0.28
// proc-macro mode, bindings are read from the compiled .so's embedded
// metadata via `uniffi-bindgen generate --library <path-to-so>`.
val generateUniFFIBindings = tasks.register<Exec>("generateUniFFIBindings") {
    group = "rust"
    description = "Run uniffi-bindgen to produce Kotlin sources from the compiled libketraffi.so."
    val outDir = file("src/main/kotlin/io/ketracopy/uniffi")
    val soFile = file("src/main/jniLibs/arm64-v8a/libketraffi.so")
    doFirst { outDir.mkdirs() }
    dependsOn(buildRustLibs)
    inputs.file(soFile)
    outputs.dir(outDir)
    // `uniffi-bindgen` must be on PATH (install via
    // `cargo install uniffi-bindgen-cli` — the GitHub Actions workflow does
    // this automatically).
    commandLine = listOf(
        "uniffi-bindgen",
        "generate",
        "--library", soFile.canonicalPath,
        "--language", "kotlin",
        "--out-dir", outDir.canonicalPath,
    )
    isIgnoreExitValue = false
}

android.applicationVariants.all {
    val variantName = this.name
    if (variantName == "debug" || variantName == "release") {
        tasks.named("merge${variantName.replaceFirstChar { it.uppercase() }}JniLibFolders") {
            dependsOn(buildRustLibs)
        }
        tasks.named("compile${variantName.replaceFirstChar { it.uppercase() }}Kotlin") {
            dependsOn(generateUniFFIBindings)
        }
    }
}

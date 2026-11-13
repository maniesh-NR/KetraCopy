// Top-level build file. Plugin versions are pinned here so the whole
// project uses a consistent toolchain (AGP 8.7.0 / Kotlin 2.0.21 / Compose
// Compiler 1.5.15 — the "stable profile" from the android-project-generator
// skill, validated against the version matrix in
// skills/android-project-generator/references/version-matrix.md).
plugins {
    id("com.android.application") version "8.7.0" apply false
    id("org.jetbrains.kotlin.android") version "2.0.21" apply false
    id("org.jetbrains.kotlin.plugin.compose") version "2.0.21" apply false
}

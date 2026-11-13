package io.ketracopy.data

import android.content.Context
import java.io.File

/// Holds the single shared `KetraEngine` instance for the app lifetime.
/// The engine is a thin Kotlin wrapper around the UniFFI-generated bindings
/// (`io.ketracopy.uniffi.ketra.KetraEngine`). The bindings land in the
/// `uniffi` package after `generateUniFFIBindings` runs.
///
/// We keep the wrapper minimal: the Android UI doesn't touch the raw
/// UniFFI types directly; instead this holder exposes a friendlier Kotlin
/// API surface (Flow-based progress, scoped to Dispatchers.IO).
object KetraEngineHolder {
    private lateinit var appContext: Context

    /// The UniFFI-generated engine. Initialized lazily so the first access
    /// (which triggers `System.loadLibrary("ketraffi")`) happens off the
    /// main thread.
    val engine: io.ketracopy.uniffi.ketra.KetraEngine by lazy {
        val alias = android.os.Build.USER ?: "Ketra"
        val model = android.os.Build.MODEL ?: "Android"
        val port = 53317
        val idDir = File(appContext.filesDir, "identity").absolutePath
        io.ketracopy.uniffi.ketra.KetraEngine(alias, model, "mobile", port.toUShort(), idDir)
    }

    fun init(context: Context) {
        appContext = context.applicationContext
    }
}

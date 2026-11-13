package io.ketracopy.provider

/// A remote file provider (WebDAV / SMB / Google Drive). Each provider
/// implements a minimal listing + openInputStream/openOutputStream API so
/// the CloudSmbScreen can browse and transfer uniformly.
///
/// We only ship a WebDAV provider here; SMB and Google Drive are stubbed
/// (they require native libs or Google Play Services that would bloat the
/// APK). The interface lets you add them later without touching the UI.
interface CloudProvider {
    val displayName: String
    val scheme: String

    /// Connect using provider-specific config. Returns true on success.
    suspend fun connect(config: Map<String, String>): Boolean

    /// List children of a path (e.g. "/Photos/2024").
    suspend fun list(path: String): List<RemoteEntry>

    /// Open an input stream for reading. Caller closes.
    suspend fun openInput(path: String): java.io.InputStream?

    /// Open an output stream for writing. Caller closes.
    suspend fun openOutput(path: String): java.io.OutputStream?

    /// Disconnect and free resources.
    fun close()
}

data class RemoteEntry(
    val name: String,
    val path: String,
    val size: Long,
    val isDirectory: Boolean,
)

package io.ketracopy.provider

import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import org.w3c.dom.Element
import java.io.InputStream
import java.io.OutputStream
import java.util.concurrent.TimeUnit
import javax.xml.parsers.DocumentBuilderFactory

/// A minimal WebDAV provider built on OkHttp + PROPFIND. Supports listing,
/// download (GET), upload (PUT), and basic auth. Enough to browse a
/// Nextcloud / Synology / Apache WebDAV share and transfer files to/from
/// it using ketra-core's accelerated pipeline.
class WebDavProvider : CloudProvider {
    override val displayName = "WebDAV"
    override val scheme = "https"

    private val client = OkHttpClient.Builder()
        .connectTimeout(15, TimeUnit.SECONDS)
        .readTimeout(60, TimeUnit.SECONDS)
        .build()
    private lateinit var baseUrl: String
    private var authHeader: String? = null

    override suspend fun connect(config: Map<String, String>): Boolean {
        baseUrl = config["url"]?.trimEnd('/') ?: return false
        val user = config["user"]
        val pass = config["pass"]
        if (!user.isNullOrEmpty() && !pass.isNullOrEmpty()) {
            val raw = "$user:$pass"
            val b64 = android.util.Base64.encodeToString(raw.toByteArray(), android.util.Base64.NO_WRAP)
            authHeader = "Basic $b64"
        }
        // Verify with a PROPFIND on the root.
        return try {
            propfind("/") != null
        } catch (_: Exception) { false }
    }

    override suspend fun list(path: String): List<RemoteEntry> {
        val doc = propfind(path) ?: return emptyList()
        val nodes = doc.getElementsByTagName("d:response")
        val out = ArrayList<RemoteEntry>()
        for (i in 0 until nodes.length) {
            val el = nodes.item(i) as Element
            val href = textOf(el, "d:href") ?: continue
            if (href.endsWith(path) || href == path) continue  // skip self
            val name = href.trimEnd('/').substringAfterLast('/').decodeURL()
            val isDir = (textOf(el, "d:collection") != null) || href.endsWith("/")
            val size = textOf(el, "lp1:getcontentlength")?.toLongOrNull() ?: 0L
            out.add(RemoteEntry(name, href, size, isDir))
        }
        return out.sortedWith(compareByDescending<RemoteEntry> { it.isDirectory }.thenBy { it.name.lowercase() })
    }

    override suspend fun openInput(path: String): InputStream? {
        val req = Request.Builder().url("$baseUrl/$path").apply { authHeader?.let { header("Authorization", it) } }.get().build()
        return client.newCall(req).execute().body?.byteStream()
    }

    override suspend fun openOutput(path: String): OutputStream? {
        // We can't return a streaming OutputStream directly via OkHttp's API
        // without buffering; for now we return a sink that buffers in memory
        // and PUTs on close. Sufficient for the typical "upload a few photos"
        // use case; large uploads should use the ketra-core pipeline instead.
        return object : java.io.ByteArrayOutputStream() {
            override fun close() {
                super.close()
                val body = toByteArray().toRequestBody("application/octet-stream".toMediaType())
                val req = Request.Builder().url("$baseUrl/$path").apply { authHeader?.let { header("Authorization", it) } }.put(body).build()
                client.newCall(req).execute().close()
            }
        }
    }

    override fun close() {
        client.connectionPool.evictAll()
    }

    private fun propfind(path: String): org.w3c.dom.Document? {
        val req = Request.Builder()
            .url("$baseUrl/${path.trimStart('/')}")
            .apply { authHeader?.let { header("Authorization", it) } }
            .header("Depth", "1")
            .method("PROPFIND", "".toRequestBody("application/xml".toMediaType()))
            .build()
        client.newCall(req).execute().use { resp ->
            if (!resp.isSuccessful) return null
            val body = resp.body?.byteStream() ?: return null
            return DocumentBuilderFactory.newInstance().apply { isNamespaceAware = true }.newDocumentBuilder().parse(body)
        }
    }

    private fun textOf(parent: Element, tag: String): String? {
        val nl = parent.getElementsByTagName(tag)
        if (nl.length == 0) return null
        return nl.item(0).textContent?.trim()?.takeIf { it.isNotEmpty() }
    }

    private fun String.decodeURL(): String =
        java.net.URLDecoder.decode(this, "UTF-8")
}

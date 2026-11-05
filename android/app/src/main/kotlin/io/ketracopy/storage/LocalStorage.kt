package io.ketracopy.storage

import android.content.Context
import android.net.Uri
import androidx.documentfile.provider.DocumentFile
import java.io.File
import java.util.zip.ZipEntry
import java.util.zip.ZipOutputStream

/// A flat view of one SAF directory's children.
data class FileEntry(
    val name: String,
    val uri: Uri,
    val size: Long,
    val isDirectory: Boolean,
)

/// SAF-backed local storage helpers. We never touch /sdcard directly — on
/// Android 10+ the Storage Access Framework is the only way to read/write
/// user-chosen directories without MANAGE_EXTERNAL_STORAGE.
object LocalStorage {

    fun list(tree: DocumentFile): List<FileEntry> {
        return tree.listFiles().map { f ->
            FileEntry(
                name = f.name ?: "(unnamed)",
                uri = f.uri,
                size = if (f.isFile) f.length() else 0L,
                isDirectory = f.isDirectory,
            )
        }.sortedWith(compareByDescending<FileEntry> { it.isDirectory }.thenBy { it.name.lowercase() })
    }

    fun humanSize(bytes: Long): String {
        val kb = 1024.0
        val mb = kb * 1024
        val gb = mb * 1024
        return when {
            bytes >= gb -> "%.1f GiB".format(bytes / gb)
            bytes >= mb -> "%.1f MiB".format(bytes / mb)
            bytes >= kb -> "%.1f KiB".format(bytes / kb)
            else -> "$bytes B"
        }
    }

    /// Write a temp ZIP of the selected entries and return the file.
    fun tempZip(context: Context): File {
        return File(context.cacheDir, "ketra-share-${System.currentTimeMillis()}.zip")
    }

    /// Zip a set of selected files (by name) from a SAF tree to a local file.
    fun zipTo(context: Context, tree: DocumentFile?, names: Set<String>, out: File) {
        ZipOutputStream(out.outputStream().buffered()).use { zos ->
            tree?.listFiles()?.forEach { f ->
                if (f.name in names && f.isFile) {
                    zos.putNextEntry(ZipEntry(f.name))
                    context.contentResolver.openInputStream(f.uri)?.use { inp ->
                        inp.copyTo(zos)
                    }
                    zos.closeEntry()
                }
            }
        }
    }

    /// Extract a ZIP into a SAF destination tree.
    fun extract(context: Context, zip: File, dest: DocumentFile) {
        java.util.zip.ZipInputStream(zip.inputStream().buffered()).use { zis ->
            var entry = zis.nextEntry
            while (entry != null) {
                val out = dest.createFile("application/octet-stream", entry.name) ?: continue
                context.contentResolver.openOutputStream(out.uri)?.use { os ->
                    zis.copyTo(os)
                }
                entry = zis.nextEntry
            }
        }
    }
}

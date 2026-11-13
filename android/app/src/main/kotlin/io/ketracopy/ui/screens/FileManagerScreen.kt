package io.ketracopy.ui.screens

import android.net.Uri
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.documentfile.provider.DocumentFile
import io.ketracopy.R
import io.ketracopy.data.KetraEngineHolder
import io.ketracopy.storage.FileEntry
import io.ketracopy.storage.LocalStorage
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun FileManagerScreen() {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var currentTree by remember { mutableStateOf<DocumentFile?>(null) }
    var entries by remember { mutableStateOf<List<FileEntry>>(emptyList()) }
    var selected by remember { mutableStateOf<Set<String>>(emptySet()) }
    var status by remember { mutableStateOf<String?>(null) }
    var showDestPicker by remember { mutableStateOf<((Uri) -> Unit)?>(null) }

    // Open a directory via SAF.
    val openTree = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocumentTree()) { uri ->
        if (uri != null) {
            currentTree = DocumentFile.fromTreeUri(context, uri)
            scope.launch {
                entries = withContext(Dispatchers.IO) { LocalStorage.list(currentTree!!) }
            }
        }
    }
    // Pick a destination directory (for copy/move).
    val pickDest = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocumentTree()) { uri ->
        if (uri != null) {
            showDestPicker?.invoke(uri)
            showDestPicker = null
        }
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(stringResource(R.string.tab_files)) },
                actions = {
                    IconButton(onClick = { openTree.launch(null) }) {
                        Icon(Icons.Filled.FolderOpen, contentDescription = "Open folder")
                    }
                },
            )
        },
    ) { inner ->
        Column(Modifier.fillMaxSize().padding(inner)) {
            // Action bar (visible when files are selected).
            if (selected.isNotEmpty()) {
                Row(
                    Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 6.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text("${selected.size} selected", style = MaterialTheme.typography.labelLarge)
                    Spacer(Modifier.weight(1f))
                    TextButton(onClick = {
                        // Compress selected into a ZIP.
                        scope.launch {
                            val out = LocalStorage.tempZip(context)
                            LocalStorage.zipTo(context, currentTree, selected, out)
                            status = "Wrote ${out.name}"
                            selected = emptySet()
                        }
                    }) { Text(stringResource(R.string.compress)) }
                    TextButton(onClick = {
                        // Copy selected to a destination dir via ketra-core.
                        showDestPicker = { destUri ->
                            scope.launch {
                                val srcs = entries.filter { it.name in selected }.map { it.uri.toString() }
                                status = withContext(Dispatchers.IO) {
                                    val r = KetraEngineHolder.engine.copy(srcs, destUri.toString(), null)
                                    "${r.ok} ok, ${r.failed} failed (${r.bytes} bytes)"
                                }
                                selected = emptySet()
                            }
                        }
                        pickDest.launch(null)
                    }) { Text("Copy") }
                    TextButton(onClick = {
                        showDestPicker = { destUri ->
                            scope.launch {
                                val srcs = entries.filter { it.name in selected }.map { it.uri.toString() }
                                status = withContext(Dispatchers.IO) {
                                    val r = KetraEngineHolder.engine.moveFiles(srcs, destUri.toString(), null)
                                    "${r.ok} moved, ${r.failed} failed"
                                }
                                selected = emptySet()
                                // Refresh the current listing.
                                currentTree?.let { t ->
                                    entries = withContext(Dispatchers.IO) { LocalStorage.list(t) }
                                }
                            }
                        }
                        pickDest.launch(null)
                    }) { Text("Move") }
                }
                Divider()
            }

            // File listing.
            if (currentTree == null) {
                Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                    Text("Tap 📁 to open a folder", style = MaterialTheme.typography.bodyMedium)
                }
            } else {
                LazyColumn(Modifier.fillMaxSize()) {
                    items(entries, key = { it.uri }) { entry ->
                        val isSel = entry.name in selected
                        ListItem(
                            headlineContent = { Text(entry.name) },
                            supportingContent = { Text(LocalStorage.humanSize(entry.size)) },
                            leadingContent = {
                                Icon(
                                    if (entry.isDirectory) Icons.Filled.Folder else Icons.Filled.InsertDriveFile,
                                    contentDescription = null,
                                )
                            },
                            trailingContent = {
                                if (isSel) Icon(Icons.Filled.CheckCircle, contentDescription = null, tint = MaterialTheme.colorScheme.primary)
                            },
                            modifier = Modifier.clickable {
                                if (entry.isDirectory) {
                                    currentTree = DocumentFile.fromTreeUri(context, entry.uri)
                                    scope.launch {
                                        entries = withContext(Dispatchers.IO) { LocalStorage.list(currentTree!!) }
                                    }
                                } else {
                                    selected = if (isSel) selected - entry.name else selected + entry.name
                                }
                            },
                        )
                    }
                }
            }

            status?.let {
                Snackbar(Modifier.padding(12.dp)) { Text(it) }
            }
        }
    }
}

// Lazy alias for clickable() — Compose Foundation's clickable is on Modifier.
private fun Modifier.clickable(onClick: () -> Unit) = this.then(androidx.compose.foundation.clickable(onClick = onClick))

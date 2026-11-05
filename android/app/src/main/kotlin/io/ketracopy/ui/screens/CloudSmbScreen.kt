package io.ketracopy.ui.screens

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import io.ketracopy.R
import io.ketracopy.provider.CloudProvider
import io.ketracopy.provider.RemoteEntry
import io.ketracopy.provider.WebDavProvider
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun CloudSmbScreen() {
    val scope = rememberCoroutineScope()
    var providers by remember { mutableStateOf<List<CloudProvider>>(emptyList()) }
    var connected by remember { mutableStateOf<CloudProvider?>(null) }
    var entries by remember { mutableStateOf<List<RemoteEntry>>(emptyList()) }
    var currentPath by remember { mutableStateOf("/") }
    var showAdd by remember { mutableStateOf(false) }
    var status by remember { mutableStateOf<String?>(null) }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(stringResource(R.string.tab_cloud)) },
                actions = {
                    IconButton(onClick = { showAdd = true }) {
                        Icon(Icons.Filled.Add, contentDescription = stringResource(R.string.add_provider))
                    }
                },
            )
        },
    ) { inner ->
        Column(Modifier.fillMaxSize().padding(inner)) {
            if (connected == null) {
                if (providers.isEmpty()) {
                    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                        Text("Tap + to add a WebDAV / SMB / Drive provider", style = MaterialTheme.typography.bodyMedium)
                    }
                } else {
                    LazyColumn(Modifier.fillMaxSize()) {
                        items(providers) { p ->
                            ListItem(
                                headlineContent = { Text(p.displayName) },
                                leadingContent = { Icon(Icons.Filled.Cloud, contentDescription = null) },
                                modifier = Modifier.androidxClickable {
                                    scope.launch {
                                        status = "Connecting…"
                                        connected = p
                                        entries = withContext(Dispatchers.IO) { p.list("/") }
                                        currentPath = "/"
                                        status = null
                                    }
                                },
                            )
                            Divider()
                        }
                    }
                }
            } else {
                // Breadcrumb + remote listing.
                Row(Modifier.fillMaxWidth().padding(12.dp), verticalAlignment = Alignment.CenterVertically) {
                    Text(currentPath, style = MaterialTheme.typography.titleMedium)
                    Spacer(Modifier.weight(1f))
                    TextButton(onClick = {
                        connected?.close()
                        connected = null
                        entries = emptyList()
                    }) { Text("Disconnect") }
                }
                LazyColumn(Modifier.fillMaxSize()) {
                    items(entries, key = { it.path }) { e ->
                        ListItem(
                            headlineContent = { Text(e.name) },
                            supportingContent = { Text("${e.size} B") },
                            leadingContent = { Icon(if (e.isDirectory) Icons.Filled.Folder else Icons.Filled.InsertDriveFile, contentDescription = null) },
                            modifier = Modifier.androidxClickable {
                                if (e.isDirectory) {
                                    currentPath = e.path
                                    scope.launch { entries = withContext(Dispatchers.IO) { connected!!.list(e.path) } }
                                }
                            },
                        )
                        Divider()
                    }
                }
            }
            status?.let { Snackbar(Modifier.padding(12.dp)) { Text(it) } }
        }
    }

    if (showAdd) {
        AddProviderDialog(
            onDismiss = { showAdd = false },
            onAdd = { url, user, pass ->
                scope.launch {
                    val p = WebDavProvider()
                    val ok = withContext(Dispatchers.IO) { p.connect(mapOf("url" to url, "user" to user, "pass" to pass)) }
                    if (ok) {
                        providers = providers + p
                        status = "WebDAV connected"
                        showAdd = false
                    } else {
                        status = "WebDAV connection failed"
                        p.close()
                    }
                }
            },
        )
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun AddProviderDialog(onDismiss: () -> Unit, onAdd: (String, String, String) -> Unit) {
    var url by remember { mutableStateOf("https://") }
    var user by remember { mutableStateOf("") }
    var pass by remember { mutableStateOf("") }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.add_provider)) },
        text = {
            Column {
                OutlinedTextField(value = url, onValueChange = { url = it }, label = { Text("Server URL") }, singleLine = true, modifier = Modifier.fillMaxWidth())
                Spacer(Modifier.height(8.dp))
                OutlinedTextField(value = user, onValueChange = { user = it }, label = { Text("Username (optional)") }, singleLine = true, modifier = Modifier.fillMaxWidth())
                Spacer(Modifier.height(8.dp))
                OutlinedTextField(value = pass, onValueChange = { pass = it }, label = { Text("Password (optional)") }, singleLine = true, keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password), modifier = Modifier.fillMaxWidth())
                Spacer(Modifier.height(12.dp))
                Text("SMB and Google Drive providers are stubbed — add them by implementing CloudProvider.",
                     style = MaterialTheme.typography.labelSmall)
            }
        },
        confirmButton = { TextButton(onClick = { onAdd(url, user, pass) }) { Text("Connect") } },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}

private fun Modifier.androidxClickable(onClick: () -> Unit) =
    this.then(androidx.compose.foundation.clickable(onClick = onClick))

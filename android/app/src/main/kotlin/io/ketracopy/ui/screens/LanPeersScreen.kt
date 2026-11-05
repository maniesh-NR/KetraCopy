package io.ketracopy.ui.screens

import android.content.Intent
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
import io.ketracopy.R
import io.ketracopy.data.KetraEngineHolder
import io.ketracopy.net.ReceiveService
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun LanPeersScreen() {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var peers by remember { mutableStateOf<List<io.ketracopy.uniffi.ketra.PeerInfo>>(emptyList()) }
    var receiverRunning by remember { mutableStateOf(false) }
    var status by remember { mutableStateOf<String?>(null) }
    var selectedFiles by remember { mutableStateOf<List<String>>(emptyList()) }
    var selectedPeer by remember { mutableStateOf<String?>(null) }

    // Start discovery as soon as the screen opens.
    LaunchedEffect(Unit) {
        withContext(Dispatchers.IO) { KetraEngineHolder.engine.startDiscovery() }
        while (true) {
            peers = withContext(Dispatchers.IO) { KetraEngineHolder.engine.listPeers() }
            delay(2000)
        }
    }
    // Stop discovery when leaving the screen.
    DisposableEffect(Unit) {
        onDispose {
            scope.launch { withContext(Dispatchers.IO) { KetraEngineHolder.engine.stopDiscovery() } }
        }
    }

    val pickFiles = rememberLauncherForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { uris ->
        if (uris.isNotEmpty()) {
            selectedFiles = uris.map { it.toString() }
            status = "${uris.size} file(s) selected — pick a peer below"
        }
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(stringResource(R.string.tab_lan)) },
                actions = {
                    IconButton(onClick = {
                        selectedPeer = null
                        pickFiles.launch(arrayOf("*/*"))
                    }) {
                        Icon(Icons.Filled.Send, contentDescription = "Send files")
                    }
                    IconButton(onClick = {
                        if (receiverRunning) {
                            context.stopService(Intent(context, ReceiveService::class.java))
                            receiverRunning = false
                        } else {
                            val dest = android.os.Environment.getExternalStoragePublicDirectory(
                                android.os.Environment.DIRECTORY_DOWNLOADS
                            ).absolutePath
                            context.startForegroundService(Intent(context, ReceiveService::class.java).putExtra("dest", dest))
                            receiverRunning = true
                        }
                    }) {
                        Icon(
                            if (receiverRunning) Icons.Filled.Stop else Icons.Filled.Download,
                            contentDescription = "Receive",
                        )
                    }
                },
            )
        },
    ) { inner ->
        Column(Modifier.fillMaxSize().padding(inner)) {
            if (receiverRunning) {
                ListItem(
                    headlineContent = { Text(stringResource(R.string.receiver_running, 53317)) },
                    leadingContent = { Icon(Icons.Filled.Wifi, contentDescription = null, tint = MaterialTheme.colorScheme.primary) },
                )
                Divider()
            }
            if (peers.isEmpty()) {
                Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                    Text(stringResource(R.string.no_peers), style = MaterialTheme.typography.bodyMedium)
                }
            } else {
                Text(
                    stringResource(R.string.peers_found, peers.size),
                    style = MaterialTheme.typography.labelLarge,
                    modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                )
                LazyColumn(Modifier.fillMaxSize()) {
                    items(peers, key = { it.fingerprint }) { peer ->
                        val isSel = selectedPeer == peer.fingerprint
                        ListItem(
                            headlineContent = { Text(peer.alias) },
                            supportingContent = { Text("${peer.deviceType} · ${peer.fingerprint.take(12)}…") },
                            leadingContent = { Icon(if (peer.ketra) Icons.Filled.Bolt else Icons.Filled.Devices, contentDescription = null) },
                            trailingContent = {
                                if (isSel) Icon(Icons.Filled.CheckCircle, contentDescription = null, tint = MaterialTheme.colorScheme.primary)
                            },
                            modifier = Modifier.androidxClickable {
                                if (selectedFiles.isEmpty()) {
                                    status = "Pick files first (📎 icon)"
                                } else {
                                    selectedPeer = peer.fingerprint
                                    scope.launch {
                                        status = withContext(Dispatchers.IO) {
                                            val r = KetraEngineHolder.engine.send(
                                                selectedFiles, peer.fingerprint, null, null,
                                            )
                                            "${r.ok} sent, ${r.failed} failed (${r.bytes} bytes)"
                                        }
                                        selectedFiles = emptyList()
                                        selectedPeer = null
                                    }
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

// Compose Foundation's clickable is an extension on Modifier; this alias
// avoids name clashes with the one in FileManagerScreen.
private fun Modifier.androidxClickable(onClick: () -> Unit) =
    this.then(androidx.compose.foundation.clickable(onClick = onClick))

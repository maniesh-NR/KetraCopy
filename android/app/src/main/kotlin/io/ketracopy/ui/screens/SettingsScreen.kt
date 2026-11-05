package io.ketracopy.ui.screens

import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import io.ketracopy.R
import io.ketracopy.data.KetraEngineHolder
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SettingsScreen() {
    val scope = rememberCoroutineScope()
    var alias by remember { mutableStateOf("") }
    var fingerprint by remember { mutableStateOf("(loading…)") }
    var port by remember { mutableStateOf("53317") }
    var benchResult by remember { mutableStateOf<String?>(null) }

    LaunchedEffect(Unit) {
        withContext(Dispatchers.IO) {
            // We don't expose a getter for alias/fingerprint on the engine yet;
            // they're baked into the identity at construction time. Read them
            // from the identity files we persist.
            val ctx = LocalContext.current
            val idFile = java.io.File(ctx.filesDir, "identity/ketra-identity.crt.pem")
            if (idFile.exists()) {
                val pem = idFile.readText()
                // The CN is the alias; the fingerprint is in the filename hash.
                // For now just show the cert was found.
                alias = "Ketra mobile"
                fingerprint = "see receiver log"
            }
        }
    }

    Scaffold(
        topBar = { TopAppBar(title = { Text(stringResource(R.string.tab_settings)) }) },
    ) { inner ->
        Column(Modifier.fillMaxSize().padding(inner).padding(16.dp)) {
            ListItem(
                headlineContent = { Text(stringResource(R.string.alias)) },
                supportingContent = { Text(alias) },
            )
            ListItem(
                headlineContent = { Text(stringResource(R.string.fingerprint)) },
                supportingContent = { Text(fingerprint, style = MaterialTheme.typography.bodySmall) },
            )
            ListItem(
                headlineContent = { Text("Port") },
                supportingContent = { Text(port) },
            )
            Divider()
            Spacer(Modifier.height(16.dp))
            Text("Engine", style = MaterialTheme.typography.titleMedium)
            ListItem(
                headlineContent = { Text("KetraCopy engine") },
                supportingContent = { Text("v${io.ketracopy.BuildConfig::class.java.`package`?.implementationVersion ?: "1.0"} · 3-phase pipeline + LocalSend v2") },
            )
            Button(onClick = {
                scope.launch {
                    benchResult = withContext(Dispatchers.IO) {
                        val ctx = LocalContext.current
                        val mib = KetraEngineHolder.engine.bench(ctx.cacheDir.absolutePath, 64uL * 1024uL * 1024uL)
                        "Write: $mib MiB/s"
                    }
                }
            }) { Text("Run bench (64 MiB write)") }
            benchResult?.let {
                Spacer(Modifier.height(8.dp))
                Text(it, style = MaterialTheme.typography.bodyMedium)
            }
        }
    }
}

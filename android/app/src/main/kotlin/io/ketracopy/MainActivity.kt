package io.ketracopy

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Cloud
import androidx.compose.material.icons.filled.Devices
import androidx.compose.material.icons.filled.Folder
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material3.Icon
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.navigation.NavDestination.Companion.hierarchy
import androidx.navigation.NavGraph.Companion.findStartDestination
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.currentBackStackEntryAsState
import androidx.navigation.compose.rememberNavController
import io.ketracopy.data.KetraEngineHolder
import io.ketracopy.ui.screens.CloudSmbScreen
import io.ketracopy.ui.screens.FileManagerScreen
import io.ketracopy.ui.screens.LanPeersScreen
import io.ketracopy.ui.screens.SettingsScreen
import io.ketracopy.ui.theme.KetraCopyTheme

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        // The shared KetraEngine lives for the whole app session — created
        // here so the foreground service and the UI share the same peer
        // registry + identity.
        KetraEngineHolder.init(applicationContext)
        setContent {
            KetraCopyTheme {
                KetraApp()
            }
        }
    }
}

@Composable
fun KetraApp() {
    val navController = rememberNavController()
    val tabs = remember {
        listOf(
            Tab("files", R.string.tab_files, Icons.Filled.Folder),
            Tab("lan", R.string.tab_lan, Icons.Filled.Devices),
            Tab("cloud", R.string.tab_cloud, Icons.Filled.Cloud),
            Tab("settings", R.string.tab_settings, Icons.Filled.Settings),
        )
    }
    Scaffold(
        bottomBar = {
            NavigationBar {
                val backStack by navController.currentBackStackEntryAsState()
                val current = backStack?.destination
                tabs.forEach { tab ->
                    NavigationBarItem(
                        icon = { Icon(tab.icon, contentDescription = null) },
                        label = { Text(stringResource(tab.labelRes)) },
                        selected = current?.hierarchy?.any { it.route == tab.route } == true,
                        onClick = {
                            navController.navigate(tab.route) {
                                popUpTo(navController.graph.findStartDestination().id) { saveState = true }
                                launchSingleTop = true
                                restoreState = true
                            }
                        },
                    )
                }
            }
        },
    ) { inner ->
        NavHost(
            navController,
            startDestination = "files",
            modifier = Modifier.fillMaxSize().padding(inner),
        ) {
            composable("files") { FileManagerScreen() }
            composable("lan") { LanPeersScreen() }
            composable("cloud") { CloudSmbScreen() }
            composable("settings") { SettingsScreen() }
        }
    }
}

private data class Tab(val route: String, val labelRes: Int, val icon: androidx.compose.ui.graphics.vector.ImageVector)

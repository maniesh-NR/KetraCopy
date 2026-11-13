package io.ketracopy.net

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.IBinder
import androidx.core.app.NotificationCompat
import io.ketracopy.R
import io.ketracopy.data.KetraEngineHolder
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch

/// Foreground service that keeps the LAN receiver alive while the user is
/// outside the app. Mirrors the desktop `ketra receive` subcommand: binds
/// the HTTPS listener on port 53317, announces via mDNS, and writes
/// incoming files into the destination directory.
class ReceiveService : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    override fun onCreate() {
        super.onCreate()
        ensureChannel()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val dest = intent?.getStringExtra("dest") ?: getExternalFilesDir(null)?.absolutePath.orEmpty()
        startForeground(NOTIF_ID, buildNotification("Listening on :53317"))
        scope.launch {
            KetraEngineHolder.engine.startReceiver(dest, true, null)
        }
        return START_STICKY
    }

    override fun onDestroy() {
        KetraEngineHolder.engine.stopReceiver()
        scope.cancel()
        super.onDestroy()
    }

    override fun onBind(intent: Intent?): IBinder? = null

    private fun ensureChannel() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val mgr = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
            if (mgr.getNotificationChannel(CHANNEL_ID) == null) {
                mgr.createNotificationChannel(
                    NotificationChannel(CHANNEL_ID, getString(R.string.channel_transfer), NotificationManager.IMPORTANCE_LOW).apply {
                        description = getString(R.string.channel_transfer_desc)
                    },
                )
            }
        }
    }

    private fun buildNotification(text: String): Notification =
        NotificationCompat.Builder(this, CHANNEL_ID)
            .setSmallIcon(android.R.drawable.stat_sys_download)
            .setContentTitle(getString(R.string.app_name))
            .setContentText(text)
            .setOngoing(true)
            .build()

    companion object {
        private const val CHANNEL_ID = "ketra-transfer"
        private const val NOTIF_ID = 42
    }
}

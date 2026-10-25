package dev.freelife.mobile

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat

/**
 * Keeps clipboard sync alive while FreeLife is in the background. It shows a
 * persistent notification with a "Sync clipboard" action, which is the
 * fallback on Android 10+: background apps can't read the clipboard, so the
 * action opens [ClipboardActivity] to read the current copy while focused.
 */
class ClipboardSyncService : Service() {
    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        ensureChannel()
        val notification = buildNotification()
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            ServiceCompat.startForeground(
                this,
                NOTIFICATION_ID,
                notification,
                ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE,
            )
        } else {
            startForeground(NOTIFICATION_ID, notification)
        }
        return START_STICKY
    }

    private fun buildNotification(): Notification {
        val sync = PendingIntent.getActivity(
            this,
            0,
            Intent(this, ClipboardActivity::class.java).setAction(ClipboardActivity.ACTION_READ),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val open = PendingIntent.getActivity(
            this,
            1,
            Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        return NotificationCompat.Builder(this, CHANNEL_ID)
            .setSmallIcon(android.R.drawable.stat_notify_sync)
            .setContentTitle("Clipboard sync is on")
            .setContentText(
                "Android only lets FreeLife read copies while it's on screen. " +
                    "Tap Sync clipboard to send the current one.",
            )
            .setStyle(
                NotificationCompat.BigTextStyle().bigText(
                    "Android only lets FreeLife read copies while it's on screen. " +
                        "Tap Sync clipboard to send the current one.",
                ),
            )
            .setContentIntent(open)
            .addAction(android.R.drawable.ic_menu_share, "Sync clipboard", sync)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setCategory(NotificationCompat.CATEGORY_SERVICE)
            .build()
    }

    private fun ensureChannel() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
        val manager = getSystemService(NotificationManager::class.java)
        if (manager.getNotificationChannel(CHANNEL_ID) == null) {
            manager.createNotificationChannel(
                NotificationChannel(CHANNEL_ID, "Clipboard sync", NotificationManager.IMPORTANCE_MIN).apply {
                    description = "Shows that clipboard sync is running"
                    setShowBadge(false)
                },
            )
        }
    }

    companion object {
        private const val CHANNEL_ID = "clipboard-sync"
        private const val NOTIFICATION_ID = 4201

        fun start(context: Context) {
            val intent = Intent(context, ClipboardSyncService::class.java)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                context.startForegroundService(intent)
            } else {
                context.startService(intent)
            }
        }

        fun stop(context: Context) {
            context.stopService(Intent(context, ClipboardSyncService::class.java))
        }
    }
}

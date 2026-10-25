package dev.freelife.mobile

import android.app.Activity
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.ClipData
import android.content.Intent
import android.os.Build
import android.os.Bundle
import android.widget.Toast
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat

/**
 * A transparent activity with no UI. Android 10+ only lets an app read or
 * write the clipboard while it has focus, so this activity does the work
 * and finishes immediately:
 *  - ACTION_READ: from the sync notification, reads the current copy and
 *    sends it to paired devices.
 *  - ACTION_WRITE: from a "tap to copy" notification, applies a queued
 *    received clip.
 */
class ClipboardActivity : Activity() {
    private var handled = false

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        // Focus is what grants clipboard access, so act only once we have it.
        if (!hasFocus || handled) return
        handled = true
        when (intent?.action) {
            ACTION_READ -> {
                if (!ClipboardChannel.readFromNotification()) {
                    toast("Open FreeLife once to turn on clipboard sync")
                }
            }

            ACTION_WRITE -> {
                val pending = pendingWrite
                val from = pendingDevice ?: "another device"
                pendingWrite = null
                pendingDevice = null
                if (pending != null && writeToClipboard(pending)) {
                    toast("Copied from $from")
                }
            }
        }
        finish()
    }

    private fun writeToClipboard(clip: ClipData): Boolean {
        val manager = getSystemService(CLIPBOARD_SERVICE) as android.content.ClipboardManager
        manager.setPrimaryClip(clip)
        return manager.primaryClip != null
    }

    private fun toast(message: String) = Toast.makeText(this, message, Toast.LENGTH_SHORT).show()

    companion object {
        const val ACTION_READ = "dev.freelife.mobile.CLIPBOARD_READ"
        const val ACTION_WRITE = "dev.freelife.mobile.CLIPBOARD_WRITE"
        private const val CHANNEL_ID = "clipboard-received"
        private const val NOTIFICATION_ID = 4202

        private var pendingWrite: ClipData? = null
        private var pendingDevice: String? = null

        /**
         * A received clip Android refused to store from the background. Keep
         * it and offer a notification; tapping it writes the clip while focused.
         */
        fun queueWrite(context: android.content.Context, clip: ClipData, device: String) {
            queue(context, clip, device, "Tap to copy the text from $device")
        }

        fun queueImage(context: android.content.Context, clip: ClipData, device: String) {
            queue(context, clip, device, "Tap to copy the image from $device")
        }

        private fun queue(context: android.content.Context, clip: ClipData, device: String, body: String) {
            pendingWrite = clip
            pendingDevice = device
            val tap = PendingIntent.getActivity(
                context,
                2,
                Intent(context, ClipboardActivity::class.java).setAction(ACTION_WRITE),
                PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
            )
            val manager = context.getSystemService(NotificationManager::class.java)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O && manager.getNotificationChannel(CHANNEL_ID) == null) {
                manager.createNotificationChannel(
                    NotificationChannel(CHANNEL_ID, "Received clipboard", NotificationManager.IMPORTANCE_DEFAULT),
                )
            }
            val notification = NotificationCompat.Builder(context, CHANNEL_ID)
                .setSmallIcon(android.R.drawable.stat_sys_download_done)
                .setContentTitle("Clipboard received from $device")
                .setContentText(body)
                .setContentIntent(tap)
                .setAutoCancel(true)
                .build()
            try {
                NotificationManagerCompat.from(context).notify(NOTIFICATION_ID, notification)
            } catch (e: SecurityException) {
                // Notification permission not granted; the clip stays queued.
            }
        }
    }
}

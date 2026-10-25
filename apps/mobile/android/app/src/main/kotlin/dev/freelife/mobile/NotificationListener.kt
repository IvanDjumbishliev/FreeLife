package dev.freelife.mobile

import android.app.Notification
import android.app.PendingIntent
import android.graphics.Bitmap
import android.graphics.Canvas
import android.os.Build
import android.os.Bundle
import android.service.notification.NotificationListenerService
import android.service.notification.StatusBarNotification
import android.util.Log
import android.app.RemoteInput
import java.io.ByteArrayOutputStream
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap

class NotificationListener : NotificationListenerService() {
    private data class ReplyAction(
        val notificationId: String,
        val pendingIntent: PendingIntent,
        val remoteInputs: Array<RemoteInput>,
    )

    private val replyActions = ConcurrentHashMap<String, ReplyAction>()

    override fun onListenerConnected() {
        super.onListenerConnected()
        INSTANCE = this
    }

    override fun onListenerDisconnected() {
        if (INSTANCE === this) INSTANCE = null
        super.onListenerDisconnected()
    }

    override fun onNotificationPosted(sbn: StatusBarNotification) {
        if (sbn.packageName == packageName || sbn.isOngoing) return
        val notification = sbn.notification
        val extras = notification.extras
        val title = extras.getCharSequence(Notification.EXTRA_TITLE)?.toString().orEmpty()
        val body = (
            extras.getCharSequence(Notification.EXTRA_BIG_TEXT)
                ?: extras.getCharSequence(Notification.EXTRA_TEXT)
            )?.toString().orEmpty()
        val appName = try {
            packageManager.getApplicationLabel(
                packageManager.getApplicationInfo(sbn.packageName, 0),
            ).toString()
        } catch (error: Exception) {
            Log.w(TAG, "Could not resolve notification app label", error)
            sbn.packageName
        }

        val action = notification.actions?.firstNotNullOfOrNull { candidate ->
            val remoteInputs = candidate.remoteInputs
            val pendingIntent = candidate.actionIntent
            if (remoteInputs != null && remoteInputs.isNotEmpty() && pendingIntent != null) {
                ReplyAction(sbn.key, pendingIntent, remoteInputs)
            } else {
                null
            }
        }
        val token = action?.let {
            UUID.randomUUID().toString().also { id ->
                replyActions[id] = it
            }
        }

        NotificationBridgeChannel.emit(
            "notificationPosted",
            mapOf(
                "id" to sbn.key,
                "appPackage" to sbn.packageName,
                "appName" to appName,
                "title" to title,
                "body" to body,
                "timestamp" to sbn.postTime,
                "iconBytes" to iconBytes(sbn, notification),
                "hasReplyAction" to (token != null),
                "replyToken" to token,
                "removed" to false,
            ),
        )
    }

    override fun onNotificationRemoved(sbn: StatusBarNotification) {
        if (sbn.packageName == packageName) return
        val notification = sbn.notification
        val extras = notification.extras
        val appName = try {
            packageManager.getApplicationLabel(
                packageManager.getApplicationInfo(sbn.packageName, 0),
            ).toString()
        } catch (_: Exception) {
            sbn.packageName
        }
        replyActions.entries.removeIf { it.value.notificationId == sbn.key }
        NotificationBridgeChannel.emit(
            "notificationRemoved",
            mapOf(
                "id" to sbn.key,
                "appPackage" to sbn.packageName,
                "appName" to appName,
                "title" to (extras.getCharSequence(Notification.EXTRA_TITLE)?.toString() ?: ""),
                "body" to (
                    extras.getCharSequence(Notification.EXTRA_BIG_TEXT)
                        ?: extras.getCharSequence(Notification.EXTRA_TEXT)
                    )?.toString().orEmpty(),
                "timestamp" to sbn.postTime,
                "iconBytes" to null,
                "hasReplyAction" to false,
                "replyToken" to null,
                "removed" to true,
            ),
        )
    }

    private fun iconBytes(
        sbn: StatusBarNotification,
        notification: Notification,
    ): ByteArray? {
        return try {
            val drawable = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
                notification.getLargeIcon()?.loadDrawable(this)
            } else {
                null
            } ?: packageManager.getApplicationIcon(sbn.packageName)
            val size = 96
            val bitmap = Bitmap.createBitmap(size, size, Bitmap.Config.ARGB_8888)
            val canvas = Canvas(bitmap)
            drawable.setBounds(0, 0, size, size)
            drawable.draw(canvas)
            ByteArrayOutputStream().use { out ->
                if (!bitmap.compress(Bitmap.CompressFormat.PNG, 100, out)) return null
                out.toByteArray()
            }
        } catch (error: Exception) {
            Log.w(TAG, "Could not read notification icon", error)
            null
        }
    }

    private fun submitReply(token: String, text: String): Boolean {
        val action = replyActions[token] ?: return false
        if (text.length > 4096) return false
        return try {
            val fillInIntent = android.content.Intent()
            val results = Bundle().apply {
                action.remoteInputs.forEach { putCharSequence(it.resultKey, text) }
            }
            RemoteInput.addResultsToIntent(action.remoteInputs, fillInIntent, results)
            action.pendingIntent.send(this, 0, fillInIntent)
            replyActions.remove(token)
            true
        } catch (error: PendingIntent.CanceledException) {
            Log.w(TAG, "Notification reply action is no longer available", error)
            replyActions.remove(token)
            false
        }
    }

    private fun dismiss(id: String): Boolean {
        return try {
            cancelNotification(id)
            true
        } catch (error: Exception) {
            Log.w(TAG, "Could not dismiss notification", error)
            false
        }
    }

    companion object {
        private const val TAG = "FreeLifeNotifications"

        @Volatile
        private var INSTANCE: NotificationListener? = null

        fun reply(token: String, text: String): Boolean =
            INSTANCE?.submitReply(token, text) ?: false

        fun dismissById(id: String): Boolean =
            INSTANCE?.dismiss(id) ?: false
    }
}

package dev.freelife.mobile

import android.app.Activity
import android.content.ComponentName
import android.content.Intent
import android.provider.Settings
import android.os.Handler
import android.os.Looper
import io.flutter.plugin.common.BinaryMessenger
import io.flutter.plugin.common.MethodCall
import io.flutter.plugin.common.MethodChannel

class NotificationBridgeChannel(
    private val activity: Activity,
    messenger: BinaryMessenger,
) : MethodChannel.MethodCallHandler {
    private val channel = MethodChannel(messenger, NAME).also {
        it.setMethodCallHandler(this)
        attach(it)
    }

    override fun onMethodCall(call: MethodCall, result: MethodChannel.Result) {
        when (call.method) {
            "hasListenerAccess" -> result.success(hasListenerAccess())
            "ready" -> {
                ready()
                result.success(null)
            }
            "openListenerSettings" -> {
                activity.startActivity(Intent(Settings.ACTION_NOTIFICATION_LISTENER_SETTINGS))
                result.success(null)
            }
            "reply" -> {
                val token = call.argument<String>("replyToken")
                val text = call.argument<String>("text")
                if (token == null || text == null) {
                    result.error("BAD_ARGS", "replyToken and text are required", null)
                } else {
                    result.success(NotificationListener.reply(token, text))
                }
            }
            "dismiss" -> {
                val id = call.argument<String>("id")
                if (id == null) {
                    result.error("BAD_ARGS", "id is required", null)
                } else {
                    result.success(NotificationListener.dismissById(id))
                }
            }
            else -> result.notImplemented()
        }
    }

    private fun hasListenerAccess(): Boolean {
        val enabled = Settings.Secure.getString(
            activity.contentResolver,
            "enabled_notification_listeners",
        ).orEmpty()
        val service = ComponentName(activity, NotificationListener::class.java)
        return enabled.split(':').any { ComponentName.unflattenFromString(it) == service }
    }

    fun dispose() {
        channel.setMethodCallHandler(null)
        detach(channel)
    }

    companion object {
        const val NAME = "dev.freelife/notifications"
        private val main = Handler(Looper.getMainLooper())
        private val lock = Any()
        private var channel: MethodChannel? = null
        private var dartReady = false
        private val pending = ArrayDeque<Pair<String, Any?>>()

        private fun attach(value: MethodChannel) {
            synchronized(lock) {
                channel = value
                dartReady = false
            }
        }

        private fun detach(value: MethodChannel) {
            synchronized(lock) {
                if (channel === value) {
                    channel = null
                    dartReady = false
                }
            }
        }

        private fun ready() {
            main.post {
                val (current, queued) = synchronized(lock) {
                    dartReady = true
                    channel to pending.toList().also { pending.clear() }
                }
                current?.let { methodChannel ->
                    queued.forEach { (method, args) -> methodChannel.invokeMethod(method, args) }
                }
            }
        }

        fun emit(method: String, args: Any?) {
            main.post {
                val current = synchronized(lock) {
                    channel.takeIf { dartReady } ?: run {
                        if (pending.size == 50) pending.removeFirst()
                        pending.addLast(method to args)
                        null
                    }
                }
                current?.invokeMethod(method, args)
            }
        }
    }
}

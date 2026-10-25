package dev.freelife.mobile

import android.app.Activity
import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.util.Log
import androidx.core.content.ContextCompat
import androidx.core.content.FileProvider
import io.flutter.plugin.common.BinaryMessenger
import io.flutter.plugin.common.MethodCall
import io.flutter.plugin.common.MethodChannel
import java.io.File

/**
 * Native side of `dev.freelife/clipboard` (Dart: lib/platform/clipboard_channel.dart).
 *
 * Reads the Android clipboard and writes received content to it. Watching
 * happens in two places:
 *  - while the app is on screen, through [ClipboardManager.OnPrimaryClipChangedListener];
 *  - from the sync notification, through [ClipboardActivity], which is the
 *    only way to read the clipboard from the background on Android 10+.
 */
class ClipboardChannel(
    private val activity: Activity,
    messenger: BinaryMessenger,
) : MethodChannel.MethodCallHandler {
    private val channel = MethodChannel(messenger, NAME).also { it.setMethodCallHandler(this) }
    private val main = Handler(Looper.getMainLooper())
    private val clipboard = activity.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager

    private val listener = ClipboardManager.OnPrimaryClipChangedListener { readAndReport() }

    init {
        CURRENT = this
        clipboard.addPrimaryClipChangedListener(listener)
    }

    override fun onMethodCall(call: MethodCall, result: MethodChannel.Result) {
        when (call.method) {
            "readNow" -> {
                readAndReport()
                result.success(null)
            }

            "writeText" -> {
                val text = call.argument<String>("text")
                val device = call.argument<String>("device") ?: "another device"
                if (text == null) {
                    result.error("BAD_ARGS", "text is required", null)
                } else {
                    result.success(write(ClipData.newPlainText("FreeLife", text), text, device))
                }
            }

            "writePng" -> {
                val bytes = call.argument<ByteArray>("bytes")
                val device = call.argument<String>("device") ?: "another device"
                if (bytes == null) {
                    result.error("BAD_ARGS", "bytes is required", null)
                } else {
                    result.success(writePng(bytes, device))
                }
            }

            "startSyncService" -> {
                ClipboardSyncService.start(activity)
                result.success(null)
            }

            "stopSyncService" -> {
                ClipboardSyncService.stop(activity)
                result.success(null)
            }

            else -> result.notImplemented()
        }
    }

    /** Called from the listener or the notification's activity when the clipboard has something new. */
    fun readAndReport() {
        val clip = clipboard.primaryClip ?: return
        if (clip.itemCount == 0) return
        val item = clip.getItemAt(0)
        val marked = isMarkedSensitive(clip.description)

        val text = item.text?.toString()
        if (!text.isNullOrEmpty()) {
            emit(mapOf("kind" to "text", "text" to text, "markedSensitive" to marked))
            return
        }
        val uri = item.uri
        if (uri != null && clip.description.hasMimeType("image/png")) {
            val bytes = try {
                activity.contentResolver.openInputStream(uri)?.use { it.readBytes() }
            } catch (e: Exception) {
                // Not readable from here (e.g. another app's private image).
                Log.w(TAG, "could not read copied image", e)
                null
            }
            if (bytes != null) emit(mapOf("kind" to "png", "bytes" to bytes, "markedSensitive" to marked))
        }
    }

    private fun isMarkedSensitive(description: ClipDescription): Boolean {
        // Android 13+ sets this for copies from password managers and other
        // apps that mark clips sensitive.
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) return false
        return description.extras?.getBoolean(ClipDescription.EXTRA_IS_SENSITIVE, false) == true
    }

    private fun emit(event: Map<String, Any?>) {
        main.post { channel.invokeMethod("localClip", event) }
    }

    /**
     * Put text on the clipboard. Android only accepts writes from the app
     * with focus (or the default keyboard). If the write doesn't stick, the
     * content is queued and the user is offered a tap-to-copy notification.
     * Returns true when the clipboard now holds `text`.
     */
    fun write(clip: ClipData, text: String, device: String): Boolean {
        clipboard.setPrimaryClip(clip)
        // Background writes are dropped silently, leaving the old clip in
        // place, so compare against what we meant to store.
        val stored = clipboard.primaryClip?.getItemAt(0)?.text?.toString() == text
        if (stored) return true
        ClipboardActivity.queueWrite(activity, clip, device)
        return false
    }

    private fun writePng(bytes: ByteArray, device: String): Boolean {
        val dir = File(activity.cacheDir, "clipboard").apply { mkdirs() }
        // Replace the previous image rather than accumulating them.
        dir.listFiles()?.forEach { it.delete() }
        val file = File(dir, "clip.png").apply { writeBytes(bytes) }
        val uri = FileProvider.getUriForFile(activity, "${activity.packageName}.fileprovider", file)
        val clip = ClipData.newUri(activity.contentResolver, "FreeLife image", uri)
        clipboard.setPrimaryClip(clip)
        val stored = clipboard.primaryClip?.getItemAt(0)?.uri == uri
        if (!stored) ClipboardActivity.queueImage(activity, clip, device)
        return stored
    }

    fun dispose() {
        clipboard.removePrimaryClipChangedListener(listener)
        channel.setMethodCallHandler(null)
        if (CURRENT === this) CURRENT = null
    }

    companion object {
        const val NAME = "dev.freelife/clipboard"
        private const val TAG = "FreeLife"

        /** The live channel, if the Flutter engine is attached. */
        @Volatile
        var CURRENT: ClipboardChannel? = null
            private set

        /** Read the clipboard on behalf of the notification (see [ClipboardActivity]). */
        fun readFromNotification(): Boolean {
            val c = CURRENT ?: return false
            c.readAndReport()
            return true
        }
    }
}

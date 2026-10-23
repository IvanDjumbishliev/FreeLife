package dev.freelife.mobile

import android.Manifest
import android.app.Activity
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.net.wifi.WifiManager
import android.os.Build
import android.os.Environment
import android.os.Handler
import android.os.Looper
import android.provider.OpenableColumns
import android.util.Log
import android.webkit.MimeTypeMap
import androidx.core.app.ActivityCompat
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import androidx.core.content.FileProvider
import androidx.core.content.IntentCompat
import io.flutter.plugin.common.BinaryMessenger
import io.flutter.plugin.common.MethodCall
import io.flutter.plugin.common.MethodChannel
import java.io.File
import java.util.UUID
import java.util.concurrent.Executors

/**
 * Native side of the `dev.freelife/airshare` channel; Dart side is
 * lib/platform/airshare_channel.dart. Keep method names in sync.
 */
class AirShareChannel(
    private val activity: Activity,
    messenger: BinaryMessenger,
) : MethodChannel.MethodCallHandler {
    private val channel = MethodChannel(messenger, NAME).also { it.setMethodCallHandler(this) }
    private val io = Executors.newSingleThreadExecutor()
    private val main = Handler(Looper.getMainLooper())
    private var multicastLock: WifiManager.MulticastLock? = null

    // Shared files are queued until Dart asks for them, then pushed live.
    private var dartReady = false
    private val pendingShared = mutableListOf<String>()

    override fun onMethodCall(call: MethodCall, result: MethodChannel.Result) {
        when (call.method) {
            "getStorageDirs" -> result.success(
                mapOf(
                    "dataDir" to activity.filesDir.absolutePath,
                    "downloadDir" to downloadDir().absolutePath,
                ),
            )

            "takeSharedFiles" -> {
                dartReady = true
                result.success(pendingShared.toList())
                pendingShared.clear()
            }

            "showFileReceivedNotification" -> {
                val device = call.argument<String>("device")
                val filename = call.argument<String>("filename")
                val path = call.argument<String>("path")
                if (device == null || filename == null || path == null) {
                    result.error("BAD_ARGS", "device, filename and path are required", null)
                } else {
                    showFileReceived(device, filename, File(path))
                    result.success(null)
                }
            }

            "requestNotificationPermission" -> {
                requestNotificationPermission()
                result.success(null)
            }

            "acquireMulticastLock" -> {
                acquireMulticastLock()
                result.success(null)
            }

            else -> result.notImplemented()
        }
    }

    /** Pick up files from an ACTION_SEND / ACTION_SEND_MULTIPLE intent. */
    fun handleIntent(intent: Intent?) {
        if (intent == null || intent.getBooleanExtra(EXTRA_HANDLED, false)) return
        val uris = sharedUris(intent)
        if (uris.isEmpty()) return
        // Don't re-send the same intent if the activity is recreated.
        intent.putExtra(EXTRA_HANDLED, true)

        io.execute {
            val paths = uris.mapNotNull(::copyToCache)
            main.post {
                if (paths.isEmpty()) return@post
                if (dartReady) {
                    channel.invokeMethod("sharedFiles", paths)
                } else {
                    pendingShared += paths
                }
            }
        }
    }

    fun dispose() {
        channel.setMethodCallHandler(null)
        multicastLock?.takeIf { it.isHeld }?.release()
        io.shutdown()
    }

    private fun sharedUris(intent: Intent): List<Uri> = when (intent.action) {
        Intent.ACTION_SEND ->
            listOfNotNull(IntentCompat.getParcelableExtra(intent, Intent.EXTRA_STREAM, Uri::class.java))

        Intent.ACTION_SEND_MULTIPLE ->
            IntentCompat.getParcelableArrayListExtra(intent, Intent.EXTRA_STREAM, Uri::class.java)
                .orEmpty()

        else -> emptyList()
    }

    /**
     * Rust needs a real path, but shares arrive as content:// URIs, so copy
     * each one into the cache. Runs on the IO thread.
     */
    private fun copyToCache(uri: Uri): String? = try {
        val name = displayName(uri)
        val dir = File(activity.cacheDir, "shared/${UUID.randomUUID()}").apply { mkdirs() }
        val out = File(dir, name)
        val input = activity.contentResolver.openInputStream(uri)
            ?: throw IllegalStateException("cannot open $uri")
        input.use { src -> out.outputStream().use { dst -> src.copyTo(dst) } }
        out.absolutePath
    } catch (e: Exception) {
        Log.w(TAG, "Could not read shared file $uri", e)
        null
    }

    private fun displayName(uri: Uri): String {
        val queried = activity.contentResolver
            .query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)
            ?.use { c -> if (c.moveToFirst()) c.getString(0) else null }
        val name = queried ?: uri.lastPathSegment ?: "shared-file"
        // The Rust side sanitizes again; this just keeps it a single path segment.
        return name.replace('/', '_').replace('\\', '_').ifBlank { "shared-file" }
    }

    /** App-specific external Downloads: needs no storage permission. */
    private fun downloadDir(): File {
        val base = activity.getExternalFilesDir(Environment.DIRECTORY_DOWNLOADS)
            ?: File(activity.filesDir, Environment.DIRECTORY_DOWNLOADS)
        return File(base, "FreeLife")
    }

    private fun showFileReceived(device: String, filename: String, file: File) {
        if (!canNotify()) return
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val manager = activity.getSystemService(NotificationManager::class.java)
            manager.createNotificationChannel(
                NotificationChannel(
                    NOTIFICATION_CHANNEL,
                    "Received files",
                    NotificationManager.IMPORTANCE_DEFAULT,
                ),
            )
        }

        val text = "File received from $device: $filename. Tap to open."
        val builder = NotificationCompat.Builder(activity, NOTIFICATION_CHANNEL)
            .setSmallIcon(android.R.drawable.stat_sys_download_done)
            .setContentTitle("FreeLife")
            .setContentText(text)
            .setStyle(NotificationCompat.BigTextStyle().bigText(text))
            .setAutoCancel(true)
        openFileIntent(file)?.let { builder.setContentIntent(it) }

        try {
            NotificationManagerCompat.from(activity).notify(file.absolutePath.hashCode(), builder.build())
        } catch (e: SecurityException) {
            Log.w(TAG, "Notification permission revoked", e)
        }
    }

    private fun openFileIntent(file: File): PendingIntent? = try {
        val uri = FileProvider.getUriForFile(activity, "${activity.packageName}.fileprovider", file)
        val mime = MimeTypeMap.getSingleton()
            .getMimeTypeFromExtension(file.extension.lowercase())
            ?: "*/*"
        val view = Intent(Intent.ACTION_VIEW)
            .setDataAndType(uri, mime)
            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
        PendingIntent.getActivity(
            activity,
            file.absolutePath.hashCode(),
            view,
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
    } catch (e: IllegalArgumentException) {
        // The file is outside the paths in res/xml/file_paths.xml.
        Log.w(TAG, "Cannot share $file via FileProvider", e)
        null
    }

    private fun canNotify(): Boolean =
        Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU ||
            ContextCompat.checkSelfPermission(activity, Manifest.permission.POST_NOTIFICATIONS) ==
            PackageManager.PERMISSION_GRANTED

    private fun requestNotificationPermission() {
        if (!canNotify()) {
            ActivityCompat.requestPermissions(
                activity,
                arrayOf(Manifest.permission.POST_NOTIFICATIONS),
                REQUEST_NOTIFICATIONS,
            )
        }
    }

    private fun acquireMulticastLock() {
        if (multicastLock?.isHeld == true) return
        val wifi = activity.applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager
        multicastLock = wifi.createMulticastLock("freelife-mdns").apply {
            setReferenceCounted(false)
            acquire()
        }
    }

    companion object {
        const val NAME = "dev.freelife/airshare"
        private const val TAG = "FreeLife"
        private const val NOTIFICATION_CHANNEL = "airshare-received"
        private const val EXTRA_HANDLED = "dev.freelife.mobile.SHARE_HANDLED"
        private const val REQUEST_NOTIFICATIONS = 1001
    }
}

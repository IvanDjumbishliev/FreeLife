package dev.freelife.mobile

import android.Manifest
import android.app.Activity
import android.content.pm.PackageManager
import android.os.Handler
import android.os.Looper
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import io.flutter.plugin.common.BinaryMessenger
import io.flutter.plugin.common.MethodCall
import io.flutter.plugin.common.MethodChannel

/**
 * Native side of `dev.freelife/camera` (Dart: lib/platform/camera_channel.dart).
 *
 * Methods Dart calls: start, stop. Callbacks Dart receives: encodedFrame,
 * streamError, streamStopped. Frames are delivered on the main thread.
 */
class CameraChannel(
    private val activity: Activity,
    messenger: BinaryMessenger,
) : MethodChannel.MethodCallHandler {
    private val channel = MethodChannel(messenger, NAME).also { it.setMethodCallHandler(this) }
    private val main = Handler(Looper.getMainLooper())

    private val listener = object : CameraStream.Listener {
        override fun onFrame(data: ByteArray, keyframe: Boolean, timestampUs: Long) {
            main.post {
                channel.invokeMethod(
                    "encodedFrame",
                    mapOf("bytes" to data, "keyframe" to keyframe, "timestampUs" to timestampUs),
                )
            }
        }

        override fun onError(message: String) {
            main.post {
                CameraStreamService.stop(activity)
                channel.invokeMethod("streamError", mapOf("message" to message))
            }
        }

        override fun onStopped() {
            main.post {
                CameraStreamService.stop(activity)
                channel.invokeMethod("streamStopped", null)
            }
        }
    }

    override fun onMethodCall(call: MethodCall, result: MethodChannel.Result) {
        when (call.method) {
            "start" -> {
                if (ContextCompat.checkSelfPermission(activity, Manifest.permission.CAMERA) !=
                    PackageManager.PERMISSION_GRANTED
                ) {
                    ActivityCompat.requestPermissions(activity, arrayOf(Manifest.permission.CAMERA), REQUEST_CAMERA)
                    result.error("PERMISSION_NEEDED", "Allow camera access, then turn the webcam on again.", null)
                    return
                }
                try {
                    CameraStream.start(activity, listener)
                    CameraStreamService.start(activity)
                    result.success(
                        mapOf(
                            "width" to CameraStream.WIDTH,
                            "height" to CameraStream.HEIGHT,
                            "fps" to CameraStream.FPS,
                        ),
                    )
                } catch (e: Exception) {
                    result.error("CAMERA_UNAVAILABLE", e.message ?: "The camera couldn't start", null)
                }
            }

            "stop" -> {
                CameraStream.stop()
                CameraStreamService.stop(activity)
                result.success(null)
            }

            else -> result.notImplemented()
        }
    }

    fun dispose() {
        channel.setMethodCallHandler(null)
        CameraStream.stop()
    }

    companion object {
        const val NAME = "dev.freelife/camera"
        const val REQUEST_CAMERA = 2001
    }
}

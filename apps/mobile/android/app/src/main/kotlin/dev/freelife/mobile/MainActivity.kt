package dev.freelife.mobile

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.os.BatteryManager
import android.os.Build
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodCall
import io.flutter.plugin.common.MethodChannel

class MainActivity : FlutterActivity() {
    private var airShare: AirShareChannel? = null
    private var clipboard: ClipboardChannel? = null
    private var camera: CameraChannel? = null

    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)
        val messenger = flutterEngine.dartExecutor.binaryMessenger
        MethodChannel(messenger, DEVICE_CHANNEL).setMethodCallHandler(::onDeviceCall)
        airShare = AirShareChannel(this, messenger).also { it.handleIntent(intent) }
        clipboard = ClipboardChannel(this, messenger)
        camera = CameraChannel(this, messenger)
    }

    // launchMode="singleTop": shares while we're open arrive here.
    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        airShare?.handleIntent(intent)
    }

    override fun onDestroy() {
        airShare?.dispose()
        airShare = null
        clipboard?.dispose()
        clipboard = null
        camera?.dispose()
        camera = null
        super.onDestroy()
    }

    // Keep method names in sync with lib/platform/device_channel.dart.
    private fun onDeviceCall(call: MethodCall, result: MethodChannel.Result) {
        when (call.method) {
            "getDeviceInfo" -> result.success(
                mapOf(
                    "manufacturer" to Build.MANUFACTURER,
                    "model" to Build.MODEL,
                    "osVersion" to Build.VERSION.RELEASE,
                ),
            )

            "getBatteryLevel" -> {
                val battery = getSystemService(Context.BATTERY_SERVICE) as BatteryManager
                val level = battery.getIntProperty(BatteryManager.BATTERY_PROPERTY_CAPACITY)
                if (level >= 0) result.success(level) else result.success(null)
            }

            "getClipboardText" -> {
                val text = clipboard().primaryClip
                    ?.takeIf { it.itemCount > 0 }
                    ?.getItemAt(0)
                    ?.coerceToText(this)
                    ?.toString()
                result.success(text)
            }

            "setClipboardText" -> {
                val text = call.argument<String>("text")
                if (text == null) {
                    result.error("BAD_ARGS", "Missing 'text' argument", null)
                } else {
                    clipboard().setPrimaryClip(ClipData.newPlainText("FreeLife", text))
                    result.success(null)
                }
            }

            else -> result.notImplemented()
        }
    }

    private fun clipboard() = getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager

    companion object {
        private const val DEVICE_CHANNEL = "dev.freelife/device"
    }
}

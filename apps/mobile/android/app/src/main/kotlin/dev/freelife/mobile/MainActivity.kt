package dev.freelife.mobile

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.os.BatteryManager
import android.os.Build
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodCall
import io.flutter.plugin.common.MethodChannel

class MainActivity : FlutterActivity() {
    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)
        MethodChannel(flutterEngine.dartExecutor.binaryMessenger, DEVICE_CHANNEL)
            .setMethodCallHandler(::onDeviceCall)
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

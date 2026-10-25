package dev.freelife.mobile

import android.content.Context
import android.graphics.ImageFormat
import android.hardware.camera2.CameraCaptureSession
import android.hardware.camera2.CameraCharacteristics
import android.hardware.camera2.CameraDevice
import android.hardware.camera2.CameraManager
import android.hardware.camera2.CaptureRequest
import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaFormat
import android.os.Handler
import android.os.HandlerThread
import android.os.SystemClock
import android.util.Range
import android.view.Surface

/**
 * ContinuityCamera capture: the rear camera (Camera2) rendered straight into
 * a MediaCodec H.264 encoder's input surface. Each encoded access unit goes
 * to [Listener]. Frame size and rate match the desktop's expectations:
 * 1920x1080 at 30 fps.
 *
 * One stream at a time, process-wide. The Flutter side reads frames through
 * [CameraChannel] and passes them to Rust.
 */
object CameraStream {
    interface Listener {
        fun onFrame(data: ByteArray, keyframe: Boolean, timestampUs: Long)
        fun onError(message: String)
        /** The stream ended, on request or because the camera went away. */
        fun onStopped()
    }

    const val WIDTH = 1920
    const val HEIGHT = 1080
    const val FPS = 30
    private const val BITRATE = 6_000_000
    private const val MIME = "video/avc"

    @Volatile
    private var running = false
    private var camera: CameraDevice? = null
    private var session: CameraCaptureSession? = null
    private var codec: MediaCodec? = null
    private var inputSurface: Surface? = null
    private var cameraThread: HandlerThread? = null
    private var outputThread: Thread? = null
    private var listener: Listener? = null

    /** SPS and PPS, prepended to keyframes so a late subscriber can start decoding. */
    private var codecConfig: ByteArray? = null

    /** Camera timestamps are on the boot clock; this converts them to Unix time. */
    private var epochOffsetUs = 0L

    val isRunning: Boolean get() = running

    /** Start capture and encoding. Throws if no rear camera or the encoder is unavailable. */
    @Synchronized
    fun start(context: Context, listener: Listener) {
        if (running) return
        val manager = context.getSystemService(CameraManager::class.java)
        val cameraId = manager.cameraIdList.firstOrNull { id ->
            manager.getCameraCharacteristics(id).get(CameraCharacteristics.LENS_FACING) ==
                CameraCharacteristics.LENS_FACING_BACK
        } ?: throw IllegalStateException("This phone has no rear camera")

        val format = MediaFormat.createVideoFormat(MIME, WIDTH, HEIGHT).apply {
            setInteger(MediaFormat.KEY_COLOR_FORMAT, MediaCodecInfo.CodecCapabilities.COLOR_FormatSurface)
            setInteger(MediaFormat.KEY_BIT_RATE, BITRATE)
            setInteger(MediaFormat.KEY_FRAME_RATE, FPS)
            // One keyframe a second: a new desktop subscriber waits at most that long.
            setInteger(MediaFormat.KEY_I_FRAME_INTERVAL, 1)
        }
        val encoder = MediaCodec.createEncoderByType(MIME)
        encoder.configure(format, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
        val surface = encoder.createInputSurface()
        encoder.start()

        this.listener = listener
        codec = encoder
        inputSurface = surface
        codecConfig = null
        epochOffsetUs = System.currentTimeMillis() * 1000 - SystemClock.elapsedRealtime() * 1000
        running = true

        val thread = HandlerThread("freelife-camera").apply { start() }
        cameraThread = thread
        val handler = Handler(thread.looper)
        try {
            manager.openCamera(cameraId, object : CameraDevice.StateCallback() {
                override fun onOpened(device: CameraDevice) {
                    camera = device
                    startSession(device, surface, handler)
                }

                override fun onDisconnected(device: CameraDevice) {
                    device.close()
                    fail("The camera was disconnected")
                }

                override fun onError(device: CameraDevice, error: Int) {
                    device.close()
                    fail("The camera failed (error $error)")
                }
            }, handler)
        } catch (e: SecurityException) {
            stop()
            throw IllegalStateException("Camera permission is needed", e)
        }
        startOutput(encoder)
    }

    private fun startSession(device: CameraDevice, surface: Surface, handler: Handler) {
        device.createCaptureSession(listOf(surface), object : CameraCaptureSession.StateCallback() {
            override fun onConfigured(s: CameraCaptureSession) {
                session = s
                val request = device.createCaptureRequest(CameraDevice.TEMPLATE_RECORD).apply {
                    addTarget(surface)
                    set(CaptureRequest.CONTROL_AE_TARGET_FPS_RANGE, Range(FPS, FPS))
                    set(CaptureRequest.CONTROL_MODE, CaptureRequest.CONTROL_MODE_AUTO)
                }.build()
                s.setRepeatingRequest(request, null, handler)
            }

            override fun onConfigureFailed(s: CameraCaptureSession) {
                fail("The camera couldn't start streaming at ${WIDTH}x$HEIGHT")
            }
        }, handler)
    }

    private fun startOutput(encoder: MediaCodec) {
        val thread = Thread({
            val info = MediaCodec.BufferInfo()
            while (running) {
                val index = try {
                    encoder.dequeueOutputBuffer(info, 100_000)
                } catch (e: IllegalStateException) {
                    break
                }
                if (index < 0) continue
                val buffer = encoder.getOutputBuffer(index)
                if (buffer != null && info.size > 0) {
                    buffer.position(info.offset)
                    buffer.limit(info.offset + info.size)
                    val bytes = ByteArray(info.size).also { buffer.get(it) }
                    if (info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG != 0) {
                        codecConfig = bytes
                    } else {
                        val keyframe = info.flags and MediaCodec.BUFFER_FLAG_KEY_FRAME != 0
                        val payload = if (keyframe && codecConfig != null) codecConfig!! + bytes else bytes
                        listener?.onFrame(payload, keyframe, epochOffsetUs + info.presentationTimeUs)
                    }
                }
                encoder.releaseOutputBuffer(index, false)
            }
        }, "freelife-encoder")
        outputThread = thread
        thread.start()
    }

    private fun fail(message: String) {
        val l = listener
        stop()
        l?.onError(message)
    }

    /** Stop capture and encoding. Safe to call when not running. */
    @Synchronized
    fun stop() {
        if (!running && codec == null && camera == null) return
        running = false
        val l = listener
        try {
            session?.close()
        } catch (_: Exception) {
        }
        session = null
        camera?.close()
        camera = null
        outputThread?.join(1000)
        outputThread = null
        try {
            codec?.stop()
        } catch (_: IllegalStateException) {
        }
        codec?.release()
        codec = null
        inputSurface?.release()
        inputSurface = null
        cameraThread?.quitSafely()
        cameraThread = null
        listener = null
        l?.onStopped()
    }
}

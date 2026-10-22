package example.hq.spike.ui.pairing

import android.content.Context
import android.os.Handler
import android.os.Looper
import android.util.Log
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.ImageProxy
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.ContextCompat
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.compose.LocalLifecycleOwner
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean

/**
 * A camera preview that reads QR codes with ZXing. [onText] is called with the text of a code, once per
 * second at most; what to do with it is the caller's business. No frame is stored or sent anywhere.
 */
@Composable
fun QrScanner(modifier: Modifier = Modifier, onText: (String) -> Unit) {
    val context = LocalContext.current
    val owner = LocalLifecycleOwner.current
    val main = remember { Handler(Looper.getMainLooper()) }
    // The decoder runs on its own thread; what it finds is handled on the main thread, where the screen's state lives.
    val analyzer = remember { QrAnalyzer { text -> main.post { onText(text) } } }
    val executor = remember { Executors.newSingleThreadExecutor() }
    val preview = remember { PreviewView(context).apply { scaleType = PreviewView.ScaleType.FILL_CENTER } }
    DisposableEffect(owner) {
        val binding = bind(context, owner, preview, executor, analyzer)
        onDispose {
            binding.release()
            executor.shutdown()
            // A frame decoded just before the screen went away must not start a pairing after it.
            main.removeCallbacksAndMessages(null)
        }
    }
    AndroidView(factory = { preview }, modifier = modifier)
}

/** A camera binding that can be released before it has finished starting. */
private class Binding {
    @Volatile var disposed = false
    @Volatile var provider: ProcessCameraProvider? = null

    fun release() {
        disposed = true
        provider?.unbindAll()
    }
}

private fun bind(context: Context, owner: LifecycleOwner, preview: PreviewView, executor: ExecutorService, analyzer: QrAnalyzer): Binding {
    val binding = Binding()
    val future = ProcessCameraProvider.getInstance(context)
    future.addListener({
        // Left before the camera was ready, or a phone with no back camera: nothing to bind, and nothing to crash.
        if (binding.disposed) return@addListener
        try {
            val p = future.get()
            binding.provider = p
            val view = Preview.Builder().build().also { it.surfaceProvider = preview.surfaceProvider }
            val analysis = ImageAnalysis.Builder()
                .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
                .build()
                .also { it.setAnalyzer(executor, analyzer) }
            p.unbindAll()
            p.bindToLifecycle(owner, CameraSelector.DEFAULT_BACK_CAMERA, view, analysis)
        } catch (e: Exception) {
            Log.w("HQBRIDGE", "camera not started: ${e.javaClass.simpleName}")
        }
    }, ContextCompat.getMainExecutor(context))
    return binding
}

/** Decodes the brightness plane of each frame. */
private class QrAnalyzer(private val onText: (String) -> Unit) : ImageAnalysis.Analyzer {
    private val busy = AtomicBoolean(false)
    private var lastAt = 0L

    override fun analyze(image: ImageProxy) {
        image.use {
            val now = System.currentTimeMillis()
            if (now - lastAt < MIN_GAP_MS || !busy.compareAndSet(false, true)) return
            try {
                val plane = it.planes[0]
                val buffer = plane.buffer
                // The last row may lack its padding; ZXing wants rowStride bytes for every row.
                val luma = ByteArray(maxOf(buffer.remaining(), plane.rowStride * it.height)).also { bytes -> buffer.get(bytes, 0, buffer.remaining()) }
                val text = QrCode.decode(luma, plane.rowStride, it.width, it.height)
                if (text != null) {
                    lastAt = now
                    onText(text)
                }
            } finally {
                busy.set(false)
            }
        }
    }

    private companion object {
        const val MIN_GAP_MS = 1_000L
    }
}

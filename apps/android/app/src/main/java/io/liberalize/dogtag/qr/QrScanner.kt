package io.liberalize.dogtag.qr

import android.Manifest
import android.content.pm.PackageManager
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import android.util.Size
import android.view.MotionEvent
import androidx.camera.core.CameraSelector
import androidx.camera.core.FocusMeteringAction
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.ImageProxy
import androidx.camera.core.Preview
import androidx.camera.core.resolutionselector.ResolutionSelector
import androidx.camera.core.resolutionselector.ResolutionStrategy
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import com.google.mlkit.vision.barcode.BarcodeScannerOptions
import java.util.concurrent.TimeUnit
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.ContextCompat
import com.google.mlkit.vision.barcode.BarcodeScanning
import com.google.mlkit.vision.barcode.common.Barcode
import com.google.mlkit.vision.common.InputImage
import java.util.concurrent.Executors

/**
 * A live CameraX preview that runs ML Kit barcode (QR) detection and calls [onResult] with the first
 * decoded payload. Requests CAMERA permission on first use. Device-only: on a headless build the
 * preview simply shows nothing until granted, but the scan plumbing is fully wired.
 */
@Composable
fun QrScannerView(
    modifier: Modifier = Modifier,
    onResult: (String) -> Unit,
) {
    val context = LocalContext.current
    var hasPermission by remember {
        mutableStateOf(
            ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) ==
                PackageManager.PERMISSION_GRANTED
        )
    }
    val permLauncher = rememberLauncherForActivityResult(
        ActivityResultContracts.RequestPermission()
    ) { granted -> hasPermission = granted }

    LaunchedEffect(Unit) {
        if (!hasPermission) permLauncher.launch(Manifest.permission.CAMERA)
    }

    val lifecycleOwner = androidx.lifecycle.compose.LocalLifecycleOwner.current
    val analysisExecutor = remember { Executors.newSingleThreadExecutor() }
    var delivered by remember { mutableStateOf(false) }

    DisposableEffect(Unit) { onDispose { analysisExecutor.shutdown() } }

    if (hasPermission) {
        AndroidView(
            modifier = modifier.fillMaxSize(),
            factory = { ctx ->
                val previewView = PreviewView(ctx)
                val providerFuture = ProcessCameraProvider.getInstance(ctx)
                providerFuture.addListener({
                    val provider = providerFuture.get()
                    val preview = Preview.Builder().build().also {
                        it.surfaceProvider = previewView.surfaceProvider
                    }
                    val scanner = BarcodeScanning.getClient(
                        BarcodeScannerOptions.Builder().setBarcodeFormats(Barcode.FORMAT_QR_CODE).build()
                    )
                    // 1280x720 analysis (default 640x480 can't resolve a dense QR's small modules).
                    val resolution = ResolutionSelector.Builder()
                        .setResolutionStrategy(
                            ResolutionStrategy(Size(1280, 720), ResolutionStrategy.FALLBACK_RULE_CLOSEST_HIGHER_THEN_LOWER)
                        )
                        .build()
                    val analysis = ImageAnalysis.Builder()
                        .setResolutionSelector(resolution)
                        .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
                        .build().also { ia ->
                            ia.setAnalyzer(analysisExecutor) { proxy: ImageProxy ->
                                val media = proxy.image
                                if (media != null && !delivered) {
                                    val img = InputImage.fromMediaImage(media, proxy.imageInfo.rotationDegrees)
                                    scanner.process(img)
                                        .addOnSuccessListener { codes ->
                                            val qr = codes.firstOrNull { it.format == Barcode.FORMAT_QR_CODE }
                                            val value = qr?.rawValue
                                            if (value != null && !delivered) {
                                                delivered = true
                                                onResult(value)
                                            }
                                        }
                                        .addOnCompleteListener { proxy.close() }
                                } else proxy.close()
                            }
                        }
                    runCatching {
                        provider.unbindAll()
                        val camera = provider.bindToLifecycle(
                            lifecycleOwner, CameraSelector.DEFAULT_BACK_CAMERA, preview, analysis
                        )
                        // Tap-to-focus: dense QRs often need a manual focus nudge on some phones.
                        previewView.setOnTouchListener { v, ev ->
                            if (ev.action == MotionEvent.ACTION_UP) {
                                val pt = previewView.meteringPointFactory.createPoint(ev.x, ev.y)
                                val action = FocusMeteringAction.Builder(pt)
                                    .setAutoCancelDuration(3, TimeUnit.SECONDS).build()
                                camera.cameraControl.startFocusAndMetering(action)
                                v.performClick()
                            }
                            true
                        }
                    }
                }, ContextCompat.getMainExecutor(ctx))
                previewView
            },
        )
    } else {
        Box(modifier)
    }
}

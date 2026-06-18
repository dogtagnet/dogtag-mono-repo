package io.liberalize.dogtag.qr

import android.graphics.Bitmap
import android.graphics.Color
import androidx.compose.foundation.Image
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import com.google.zxing.BarcodeFormat
import com.google.zxing.qrcode.QRCodeWriter

/** Encode [content] into a QR Bitmap and render it (share-QR). */
object QrGen {
    fun bitmap(content: String, size: Int = 600): Bitmap {
        val matrix = QRCodeWriter().encode(content, BarcodeFormat.QR_CODE, size, size)
        val bmp = Bitmap.createBitmap(size, size, Bitmap.Config.ARGB_8888)
        for (x in 0 until size) for (y in 0 until size) {
            bmp.setPixel(x, y, if (matrix[x, y]) Color.BLACK else Color.WHITE)
        }
        return bmp
    }
}

@Composable
fun QrImage(content: String, modifier: Modifier = Modifier, size: Int = 600) {
    val bmp = remember(content, size) { QrGen.bitmap(content, size) }
    Image(
        bitmap = bmp.asImageBitmap(),
        contentDescription = "QR code",
        modifier = modifier,
        contentScale = ContentScale.Fit,
    )
}

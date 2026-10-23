package example.hq.spike.ui.pairing

import android.graphics.Bitmap
import android.graphics.Color
import com.google.zxing.BarcodeFormat
import com.google.zxing.DecodeHintType
import com.google.zxing.EncodeHintType
import com.google.zxing.NotFoundException
import com.google.zxing.PlanarYUVLuminanceSource
import com.google.zxing.BinaryBitmap
import com.google.zxing.common.BitMatrix
import com.google.zxing.common.HybridBinarizer
import com.google.zxing.qrcode.QRCodeReader
import com.google.zxing.qrcode.QRCodeWriter
import com.google.zxing.qrcode.decoder.ErrorCorrectionLevel

/** QR codes with ZXing (Apache-2.0). Encoding and decoding are pure, so both are tested without a camera. */
object QrCode {
    /** Quiet zone in modules; a scanner needs a border to find the code. */
    private const val QUIET_MODULES = 2
    private val DARK = Color.BLACK
    private val LIGHT = Color.WHITE

    /** The code for [text] as modules, medium error correction (a link of ~400 characters stays readable on a screen). */
    fun encode(text: String): BitMatrix = QRCodeWriter().encode(
        text, BarcodeFormat.QR_CODE, 0, 0,
        mapOf(EncodeHintType.ERROR_CORRECTION to ErrorCorrectionLevel.M, EncodeHintType.MARGIN to QUIET_MODULES, EncodeHintType.CHARACTER_SET to "UTF-8"),
    )

    /** One pixel per module; scale it up with filtering off so the edges stay sharp. */
    fun bitmap(text: String): Bitmap {
        val m = encode(text)
        val pixels = IntArray(m.width * m.height) { i -> if (m.get(i % m.width, i / m.width)) DARK else LIGHT }
        return Bitmap.createBitmap(pixels, m.width, m.height, Bitmap.Config.ARGB_8888)
    }

    /**
     * Reads a QR code from the luminance plane of a camera frame ([rowStride] bytes per row, [width] of
     * them used). Null when there is no code in the frame. QR codes are read at any rotation.
     */
    fun decode(luma: ByteArray, rowStride: Int, width: Int, height: Int): String? {
        val source = PlanarYUVLuminanceSource(luma, rowStride, height, 0, 0, width, height, false)
        val hints = mapOf(DecodeHintType.TRY_HARDER to true, DecodeHintType.POSSIBLE_FORMATS to listOf(BarcodeFormat.QR_CODE))
        return try {
            QRCodeReader().decode(BinaryBitmap(HybridBinarizer(source)), hints).text
        } catch (e: NotFoundException) {
            null
        } catch (e: com.google.zxing.ChecksumException) {
            null
        } catch (e: com.google.zxing.FormatException) {
            null
        }
    }
}

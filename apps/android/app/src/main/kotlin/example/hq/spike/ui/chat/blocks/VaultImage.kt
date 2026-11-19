package example.hq.spike.ui.chat.blocks

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.runtime.Composable
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.produceState
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import example.hq.spike.data.HqServices
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import example.hq.spike.data.suiteMedia

/** The folder of the note being read, so a picture link that is relative to the note can be resolved. Null in chat, where pictures stay blocked. */
val LocalNoteFolder = compositionLocalOf<String?> { null }

private const val STORE_PREFIX = "_media/cas/"

/** A pictures larger than this many pixels across is decoded smaller: the reader never shows more than a phone's width. */
private const val MAX_DECODED_WIDTH = 1080

/** Where [src], written relative to [folder], points inside the vault, or null for anything outside the picture store. */
fun vaultPictureOf(folder: String, src: String): String? {
    if (src.isBlank() || src.contains("://") || src.startsWith("/") || src.contains('\\')) return null
    val parts = ArrayDeque(folder.split('/').filter { it.isNotEmpty() })
    for (part in src.split('/')) {
        when (part) {
            "", "." -> Unit
            ".." -> if (parts.isEmpty()) return null else parts.removeLast()
            else -> parts.addLast(part)
        }
    }
    val path = parts.joinToString("/")
    return path.takeIf { it.startsWith(STORE_PREFIX) }
}

/** Decodes at most [MAX_DECODED_WIDTH] pixels across; a very wide picture is read at a fraction of its size. */
internal fun decodeSampled(bytes: ByteArray): Bitmap? {
    val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
    BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
    var sample = 1
    while (bounds.outWidth / (sample * 2) >= MAX_DECODED_WIDTH) sample *= 2
    return BitmapFactory.decodeByteArray(bytes, 0, bytes.size, BitmapFactory.Options().apply { inSampleSize = sample })
}

/** A picture a note links from the suite's media store; anything else stays the blocked card. */
@Composable
fun VaultImageView(src: String) {
    val folder = LocalNoteFolder.current
    val path = folder?.let { vaultPictureOf(it, src) }
    if (path == null) {
        BlockedImageView()
        return
    }
    val client = HqServices.get(LocalContext.current).client
    val picture by produceState<Result<Bitmap?>?>(null, path) {
        value = runCatching { withContext(Dispatchers.IO) { decodeSampled(client.suiteMedia(path)) } }
    }
    when (val result = picture) {
        null -> Unit
        else -> result.getOrNull()?.let {
            Image(it.asImageBitmap(), null, Modifier.fillMaxWidth(), contentScale = ContentScale.FillWidth)
        } ?: BlockedImageView()
    }
}

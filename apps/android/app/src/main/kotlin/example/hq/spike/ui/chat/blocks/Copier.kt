package example.hq.spike.ui.chat.blocks

import android.content.ClipData
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.platform.ClipEntry
import androidx.compose.ui.platform.LocalClipboard
import kotlinx.coroutines.launch

/** Puts plain text on the clipboard. */
@Composable
fun rememberCopier(): (String) -> Unit {
    val clipboard = LocalClipboard.current
    val scope = rememberCoroutineScope()
    return remember(clipboard) { { text -> scope.launch { clipboard.setClipEntry(ClipEntry(ClipData.newPlainText("HQ", text))) }; Unit } }
}

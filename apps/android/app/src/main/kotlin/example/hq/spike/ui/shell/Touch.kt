package example.hq.spike.ui.shell

import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.pointer.pointerInput

/**
 * For the root of a full-screen page drawn over the shell. A layer with nothing to tap is see-through to Compose's hit
 * testing, so a tap on its empty title bar would reach the shell's own Search and Settings buttons underneath. This gives
 * the page something that takes the touch, and leaves the buttons and lists on it to handle theirs first.
 */
fun Modifier.consumeTouches(): Modifier = pointerInput(Unit) { detectTapGestures { } }

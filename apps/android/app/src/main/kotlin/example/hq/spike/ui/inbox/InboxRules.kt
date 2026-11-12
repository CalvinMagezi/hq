package example.hq.spike.ui.inbox

/** When the answer buttons stack: from this text scale up, two buttons side by side no longer fit their words. */
const val STACK_FROM_FONT_SCALE = 1.3f

object AnswerLayout {
    fun stacked(fontScale: Float): Boolean = fontScale >= STACK_FROM_FONT_SCALE
}

object UpdateRows {
    /** A row opens when its body has more than one line, or when the one line shown was cut ([clipped], from the text layout). */
    fun expandable(body: String, clipped: Boolean): Boolean = clipped || body.lines().count { it.isNotBlank() } > 1
}

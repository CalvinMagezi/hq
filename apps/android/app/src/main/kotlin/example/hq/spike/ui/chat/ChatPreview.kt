package example.hq.spike.ui.chat

import example.hq.spike.ui.chat.blocks.Attachments
import example.hq.spike.ui.chat.blocks.Plain

/** What a chat shows under its title: its last message as one plain line, or how many files it was if it was only files. */
object ChatPreview {
    fun snippet(raw: String?, filesLabel: (Int) -> String = { "[$it]" }): String {
        val (cut, files) = Attachments.split(raw.orEmpty())
        return if (cut.isBlank() && files.isNotEmpty()) filesLabel(files.size) else Plain.preview(cut)
    }
}

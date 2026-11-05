package example.hq.spike.ui.chat

import android.content.Context

/**
 * The chats that began or continued by voice, kept on this phone. The thread list carries no source, so a thread is
 * marked when the voice sheet puts a question in it and when its saved messages are read.
 */
object SpokenThreads {
    private const val PREFS = "spoken_threads"
    private const val IDS = "ids"
    private const val KEEP = 200

    fun mark(c: Context, id: String) {
        val prefs = c.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        val ids = prefs.getStringSet(IDS, emptySet()).orEmpty()
        if (id in ids) return
        prefs.edit().putStringSet(IDS, (ids + id).toList().takeLast(KEEP).toSet()).apply()
    }

    fun all(c: Context): Set<String> = c.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getStringSet(IDS, emptySet()).orEmpty()
}

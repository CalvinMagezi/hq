package example.hq.spike.ui.notes

/** Where each note was scrolled to when its page was left, so Back returns to the same place and a fresh open starts at the top. */
class ScrollMemory {
    private val spots = mutableMapOf<String, Pair<Int, Int>>()

    fun save(path: String, index: Int, offset: Int) { spots[path] = index to offset }

    fun restore(path: String): Pair<Int, Int>? = spots[path]

    fun forget(path: String) { spots.remove(path) }
}

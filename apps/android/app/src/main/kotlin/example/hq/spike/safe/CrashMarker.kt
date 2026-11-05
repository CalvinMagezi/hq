package example.hq.spike.safe

import java.io.File

/**
 * Tells two crashed launches in a row from a slow one. A launch writes the marker first thing and clears it after its first frame;
 * a marker still there at the next launch means that launch never drew. Two such launches send the third to the safe screen.
 * Plain files in the app's files folder, so the safe-mode process reads the same state.
 */
object CrashMarker {
    private const val FILE = "launch-crashes"
    private const val OPEN = "launch-open"
    const val LIMIT = 2

    /** Called first in a launch. Counts the previous launch as crashed when its marker was never cleared; true when this launch should go to safe mode. */
    fun begin(filesDir: File): Boolean {
        val open = File(filesDir, OPEN)
        var count = read(filesDir)
        if (open.exists()) count++
        write(filesDir, count)
        if (count >= LIMIT) { open.delete(); return true }
        runCatching { open.writeText("1") }
        return false
    }

    /** Called after the first frame: this launch worked. */
    fun drawn(filesDir: File) {
        File(filesDir, OPEN).delete()
        write(filesDir, 0)
    }

    /** True while two launches in a row have crashed and the owner has not chosen to open HQ again. */
    fun tripped(filesDir: File): Boolean = read(filesDir) >= LIMIT

    fun reset(filesDir: File) {
        File(filesDir, OPEN).delete()
        write(filesDir, 0)
    }

    private fun read(filesDir: File): Int = File(filesDir, FILE).takeIf { it.isFile }?.readText()?.trim()?.toIntOrNull() ?: 0
    private fun write(filesDir: File, n: Int) { runCatching { File(filesDir, FILE).writeText(n.toString()) } }
}

package example.hq.spike.ui.shell

import example.hq.spike.R

/** How counts are worded. The sentences are in the catalogue, with no-break spaces that keep a count from splitting across lines. */
object Counts {
    private const val BADGE_MAX = 99

    /** "1 of 3 sub-tasks done", or "1 of 3 done" when [subTasks] is false. */
    fun progress(w: Words, done: Int, total: Int, subTasks: Boolean = true): String =
        w.s(if (subTasks) R.string.count_progress_sub else R.string.count_progress, done, total)

    private const val NAV_BADGE_MAX = 9

    /** The bottom bar's number: "9+" from ten up, so a pile reads as a pile and not as a precise count. */
    fun navBadge(n: Int): String = if (n > NAV_BADGE_MAX) "9+" else n.toString()

    fun badge(n: Int): String = if (n > BADGE_MAX) "99+" else n.toString()

    fun tasks(w: Words, n: Int): String = w.n(R.plurals.count_tasks, n, n)

    fun showAll(w: Words, n: Int): String = w.s(R.string.count_show_all, n)
}

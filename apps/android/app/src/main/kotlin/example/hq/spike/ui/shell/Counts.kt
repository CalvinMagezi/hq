package example.hq.spike.ui.shell

/** One place for how counts are worded: no-break spaces keep a count from splitting across lines. */
object Counts {
    const val NBSP = '\u00a0'
    private const val BADGE_MAX = 99

    /** "1 of 3 sub-tasks done", or "1 of 3 done" when [thing] is null. */
    fun progress(done: Int, total: Int, thing: String? = "sub-tasks"): String {
        val noun = thing?.let { "${NBSP}$it" }.orEmpty()
        return "$done${NBSP}of${NBSP}$total$noun${NBSP}done"
    }

    fun badge(n: Int): String = if (n > BADGE_MAX) "99+" else n.toString()

    fun plural(n: Int, one: String, many: String): String = "$n${NBSP}${if (n == 1) one else many}"

    fun tasks(n: Int): String = plural(n, "task", "tasks")

    fun showAll(n: Int): String = "Show all$NBSP$n"

    fun showMore(n: Int): String = "Show$NBSP$n${NBSP}more"
}

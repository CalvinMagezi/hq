package example.hq.spike.ui.inbox

import example.hq.spike.data.Instants

/** Rows shown under the hero card before the rest fold into "N more waiting for you". */
const val NEEDS_ROWS_MAX = 5

/** More old helper requests than this and the catch-up card offers to clear them. */
const val CATCH_UP_OVER = 5

/** A helper request is cleared in bulk only after this long: the phone cannot see whether its helper still runs, and HQ's own tidy uses the same day. */
const val HELPER_CLEAR_AFTER_HOURS = 24L

/**
 * The Needs you tab for an HQ that tells owner decisions from helper requests: the oldest decision is the hero, the others are
 * rows (at most [NEEDS_ROWS_MAX], then [folded] more), and helper requests wait in [helpers], below, never in the hero.
 */
data class NeedsLayout(val hero: InboxCard?, val rows: List<InboxCard>, val folded: Int, val helpers: List<InboxCard>) {
    companion object {
        /**
         * [focusId] is the row the owner opened in place; it takes the hero's place and the hero folds into a row.
         * [showAll] lists every row. Cards from an HQ that does not say who they are from are all decisions.
         */
        fun of(cards: List<InboxCard>, focusId: String? = null, showAll: Boolean = false): NeedsLayout {
            val (helpers, mine) = cards.partition { it.fromHelpers }
            // Urgency first (a sign-in before an approval), then the oldest.
            val oldestFirst = mine.sortedWith(compareBy({ NEEDS_ORDER.indexOf(it.kind) }, { Instants.parse(it.createdAt) ?: java.time.Instant.MAX }))
            val hero = oldestFirst.firstOrNull { it.id == focusId } ?: oldestFirst.firstOrNull()
            val others = oldestFirst.filter { it.id != hero?.id }
            val rows = if (showAll) others else others.take(NEEDS_ROWS_MAX)
            return NeedsLayout(hero, rows, others.size - rows.size, helpers)
        }

        /** The helper requests old enough to clear in bulk. */
        fun oldHelpers(cards: List<InboxCard>, now: java.time.Instant = java.time.Instant.now()): List<InboxCard> {
            val cutoff = now.minus(java.time.Duration.ofHours(HELPER_CLEAR_AFTER_HOURS))
            return cards.filter { c -> c.fromHelpers && Instants.parse(c.createdAt)?.let { it.isBefore(cutoff) } == true }
        }

        /** The catch-up card shows when more than [CATCH_UP_OVER] helper requests are over a day old. */
        fun catchUp(cards: List<InboxCard>, now: java.time.Instant = java.time.Instant.now()): Boolean = oldHelpers(cards, now).size > CATCH_UP_OVER

        /** An HQ that does not name the group sends none; the tab then lists as it always did. */
        fun grouped(cards: List<InboxCard>): Boolean = cards.any { it.group != null }
    }
}

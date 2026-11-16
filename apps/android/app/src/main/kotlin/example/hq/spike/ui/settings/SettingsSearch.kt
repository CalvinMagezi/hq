package example.hq.spike.ui.settings

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import example.hq.spike.R
import example.hq.spike.ui.firstrun.FirstRunWords
import example.hq.spike.ui.shell.Words
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext

/** One searchable item in Settings: either a Home row (page or action) or a named section inside a page. */
data class SettingsIndexEntry(
    val page: SettingsPage?,
    val rowKey: String,
    val pageName: String,
    val sectionTitle: String? = null,
    val sectionKey: String? = null,
    val aliases: List<String> = emptyList(),
)

/** Builds the single searchable index of Settings pages, wired sections, and route aliases from [SettingsWords] and [SettingsRoutes]. */
object SettingsIndex {
    data class SectionSpec(
        val page: SettingsPage,
        val sectionKey: String,
        val title: (Words) -> String,
        val extraAliases: List<String> = emptyList(),
    )

    val SECTIONS: List<SectionSpec> = listOf(
        SectionSpec(SettingsPage.Keys, "model", { it.s(R.string.settings_label_model) }),
        SectionSpec(SettingsPage.Keys, "which_answers", { it.s(R.string.settings_label_which_answers) }),
        SectionSpec(SettingsPage.Keys, "your_keys", { it.s(R.string.settings_label_your_keys) }),
        SectionSpec(SettingsPage.Keys, "storedkeys", { FirstRunWords.KEYS_TITLE }),
        SectionSpec(SettingsPage.Assistant, "right_now", { it.s(R.string.assist_group_right_now) }),
        SectionSpec(SettingsPage.Assistant, "speech", { it.s(R.string.speech_title) }),
        SectionSpec(SettingsPage.Background, "right_now", { it.s(R.string.assist_group_right_now) }),
        SectionSpec(SettingsPage.Background, "keeprunning", { it.s(R.string.settings_keep_running_section) }, listOf("keeprunning")),
        SectionSpec(SettingsPage.Safety, "origins", { it.s(R.string.settings_origins_count) }, listOf("origins")),
        SectionSpec(SettingsPage.Safety, "trusted_sites", { it.s(R.string.sites_trusted_title) }),
        SectionSpec(SettingsPage.Computers, "computers", { it.s(R.string.settings_section_computers) }),
        SectionSpec(SettingsPage.Computers, "connectedtools", { it.s(R.string.settings_tools_connected) }),
    )

    fun build(w: Words): List<SettingsIndexEntry> {
        val pageAliases = SettingsRoutes.ALIASES.entries
            .filter { (alias, _) -> alias !in SettingsRoutes.SECTION_ALIASES }
            .groupBy({ it.value }, { it.key })

        val sectionAliases = SettingsRoutes.SECTION_ALIASES.entries
            .groupBy({ (alias, sectionKey) -> SettingsRoutes.ALIASES[alias] to sectionKey }, { it.key })

        val pagesByName = SettingsWords.HOME_SPECS.associateBy { it.page }

        val pageEntries = SettingsWords.HOME_SPECS.map { spec ->
            val routeAliases = spec.page?.let { p -> listOf(p.name.lowercase()) + pageAliases[p].orEmpty() }.orEmpty()
            SettingsIndexEntry(
                page = spec.page,
                rowKey = spec.key,
                pageName = w.s(spec.titleRes),
                aliases = routeAliases,
            )
        }

        val sectionEntries = SECTIONS.mapNotNull { sec ->
            val parent = pagesByName[sec.page] ?: return@mapNotNull null
            val aliases = sectionAliases[sec.page to sec.sectionKey].orEmpty() + sec.extraAliases
            SettingsIndexEntry(
                page = sec.page,
                rowKey = parent.key,
                pageName = w.s(parent.titleRes),
                sectionTitle = sec.title(w),
                sectionKey = sec.sectionKey,
                aliases = aliases,
            )
        }

        return pageEntries + sectionEntries
    }

    fun rowLabel(w: Words, entry: SettingsIndexEntry): String =
        if (entry.sectionTitle == null) entry.pageName
        else w.s(R.string.settings_search_result_section, entry.pageName, entry.sectionTitle)
}

/** Pure, on-device prefix search over [SettingsIndex]. */
object SettingsSearch {
    const val MAX_RESULTS = 8
    private val SPLIT = Regex("[^\\p{L}\\p{N}]+")

    fun results(query: String, index: List<SettingsIndexEntry>): List<SettingsIndexEntry> {
        val tokens = words(query)
        if (tokens.isEmpty()) return emptyList()
        return index
            .mapNotNull { entry ->
                val rank = rank(tokens, entry) ?: return@mapNotNull null
                rank to entry
            }
            .sortedBy { it.first }
            .map { it.second }
            .take(MAX_RESULTS)
    }

    private fun rank(tokens: List<String>, entry: SettingsIndexEntry): Int? {
        val pageWords = words(entry.pageName)
        val sectionWords = entry.sectionTitle?.let(::words).orEmpty()
        val aliasWords = entry.aliases.flatMap(::words)
        val allWords = pageWords + sectionWords + aliasWords
        if (allWords.isEmpty()) return null
        if (!tokens.all { t -> allWords.any { w -> w.startsWith(t) } }) return null

        val isPage = entry.sectionTitle == null
        val matchesPage = tokens.all { t -> pageWords.any { it.startsWith(t) } }
        val matchesSection = sectionWords.isNotEmpty() && tokens.all { t -> sectionWords.any { it.startsWith(t) } }
        return when {
            isPage && matchesPage -> 0
            matchesSection -> 1
            matchesPage -> 2
            else -> 3
        }
    }

    private fun words(text: String): List<String> =
        text.lowercase().split(SPLIT).filter { it.isNotEmpty() }
}

/** Targets on a Settings page that an alias or search result can scroll to and briefly highlight. */
enum class ScrollTarget {
    Top,
    KeysModel,
    KeysWhichAnswers,
    KeysYourKeys,
    KeysStored,
    AssistantRightNow,
    AssistantSpeech,
    BackgroundRightNow,
    BackgroundKeepRunning,
    SafetyAllowedAddresses,
    SafetyTrustedSites,
    ComputersList,
    ComputersTools,
}

/** Pure mapping from (page, section) to the scroll target; unknown or null sections fall back to [ScrollTarget.Top]. */
object SettingsScroll {
    val MAP: Map<Pair<SettingsPage, String>, ScrollTarget> = mapOf(
        (SettingsPage.Keys to "model") to ScrollTarget.KeysModel,
        (SettingsPage.Keys to "which_answers") to ScrollTarget.KeysWhichAnswers,
        (SettingsPage.Keys to "your_keys") to ScrollTarget.KeysYourKeys,
        (SettingsPage.Keys to "storedkeys") to ScrollTarget.KeysStored,
        (SettingsPage.Assistant to "right_now") to ScrollTarget.AssistantRightNow,
        (SettingsPage.Assistant to "speech") to ScrollTarget.AssistantSpeech,
        (SettingsPage.Background to "right_now") to ScrollTarget.BackgroundRightNow,
        (SettingsPage.Background to "keeprunning") to ScrollTarget.BackgroundKeepRunning,
        (SettingsPage.Safety to "origins") to ScrollTarget.SafetyAllowedAddresses,
        (SettingsPage.Safety to "trusted_sites") to ScrollTarget.SafetyTrustedSites,
        (SettingsPage.Computers to "computers") to ScrollTarget.ComputersList,
        (SettingsPage.Computers to "connectedtools") to ScrollTarget.ComputersTools,
    )

    fun target(page: SettingsPage, section: String?): ScrollTarget {
        val key = section?.trim()?.lowercase()?.takeIf { it.isNotEmpty() } ?: return ScrollTarget.Top
        return MAP[page to key] ?: ScrollTarget.Top
    }

    /** Scroll offset in content coordinates from a child's raw root Y (plus current scroll) and the parent's root Y. */
    fun offset(rawContentY: Float, rootTop: Int): Int =
        (rawContentY.toInt() - rootTop).coerceAtLeast(0)
}

internal const val SECTION_HIGHLIGHT_MS = 1_200L

/** State machine for scrolling to a target section and briefly highlighting it without sticking on cancellation. */
internal class SectionScrollMachine(
    val target: ScrollTarget,
    val reducedMotion: Boolean,
) {
    var rootTop: Int? = null
        private set
    private val rawPositions = mutableMapOf<ScrollTarget, Float>()
    var highlighted by mutableStateOf<ScrollTarget?>(null)
    var scrolledTarget: ScrollTarget? = null
        private set
    var layoutTick by mutableIntStateOf(0)

    fun setRootTop(y: Int) {
        if (rootTop != y) {
            rootTop = y
            if (target != ScrollTarget.Top && target in rawPositions) layoutTick++
        }
    }

    fun register(section: ScrollTarget, rawContentY: Float) {
        if (rawPositions[section] != rawContentY) {
            rawPositions[section] = rawContentY
            if (section == target && rootTop != null) layoutTick++
        }
    }

    fun targetOffset(): Int? {
        val top = rootTop ?: return null
        val raw = rawPositions[target] ?: return null
        return SettingsScroll.offset(raw, top)
    }

    suspend fun runIfReady(
        scrollTo: suspend (Int) -> Unit,
        onConsumed: () -> Unit = {},
        waitHighlight: suspend () -> Unit = { delay(SECTION_HIGHLIGHT_MS) },
    ) {
        if (target == ScrollTarget.Top || scrolledTarget == target) return
        val y = targetOffset() ?: return
        highlighted = null
        try {
            scrollTo(y)
            scrolledTarget = target
            highlighted = target
            waitHighlight()
        } finally {
            withContext(NonCancellable) {
                if (highlighted == target) highlighted = null
                if (scrolledTarget == target) onConsumed()
            }
        }
    }
}

package example.hq.spike.ui.settings

/**
 * Where each `hq://settings/<screen>` name opens now that Settings has six groups. Old names still work: a page that became
 * a section opens the page that holds it, and a page one level down opens with its parent under it so Back goes up one step.
 */
object SettingsRoutes {
    /** Names that are not a page of their own any more, and the page that holds them today. */
    val ALIASES: Map<String, SettingsPage> = mapOf(
        "storedkeys" to SettingsPage.Keys,
        "speech" to SettingsPage.Assistant,
        "connectedtools" to SettingsPage.Computers,
        "tools" to SettingsPage.Computers,
        "backup" to SettingsPage.About,
        "updates" to SettingsPage.Update,
    )

    /** The page that holds each page one level down; it is the page Back returns to. */
    val PARENT: Map<SettingsPage, SettingsPage> = mapOf(
        SettingsPage.KeepRunning to SettingsPage.Background,
        SettingsPage.Origins to SettingsPage.Safety,
        SettingsPage.Noticed to SettingsPage.Technical,
        SettingsPage.SetupApp to SettingsPage.Messaging,
        SettingsPage.PairApp to SettingsPage.Messaging,
    )

    /** What each page offers one level down: the screens draw these rows and the placement test counts them. */
    val CHILDREN: Map<SettingsPage, List<SettingsPage>> = PARENT.entries.groupBy({ it.value }, { it.key })

    /** The page a link name means, or null for a name Settings does not have. */
    fun page(name: String): SettingsPage? {
        val key = name.trim().lowercase()
        return ALIASES[key] ?: SettingsPage.entries.firstOrNull { it.name.equals(key, true) }
    }

    /** The pages to push, in order, to land on [name]; empty for Home, null for an unknown name. */
    fun stack(name: String): List<SettingsPage>? {
        val page = page(name) ?: return null
        if (page == SettingsPage.Home) return emptyList()
        return listOfNotNull(PARENT[page], page)
    }

    /** Pages a link cannot open: they need a pairing or setup in progress, or are reached from a page above. */
    private val NOT_LINKABLE = setOf(SettingsPage.SetupApp, SettingsPage.PairApp, SettingsPage.Technical, SettingsPage.Noticed)

    /** Every name `hq://settings/<name>` accepts. */
    val NAMES: Set<String> get() = (SettingsPage.entries - NOT_LINKABLE).map { it.name.lowercase() }.toSet() + ALIASES.keys
}

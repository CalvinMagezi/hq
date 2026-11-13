package example.hq.spike.ui.settings

/** The question "Remove this key?" while it is open. [only] is true when it is the last AI key, which changes the words. */
data class KeyRemove(val name: String, val only: Boolean, val busy: Boolean = false)

/**
 * The states of removing a stored key, kept on [SettingsUi] so the question is a modal over Settings and not a box inside a scrolling section.
 * Each step is a pure function of the state, so it can be tested without a phone.
 */
object KeyRemoval {
    fun ask(ui: SettingsUi, name: String, only: Boolean): SettingsUi = ui.copy(keyRemove = KeyRemove(name, only), keysLine = null)

    /** "Keep it": closes the question, unless the removal is already running. */
    fun cancel(ui: SettingsUi): SettingsUi = if (ui.keyRemove?.busy == true) ui else ui.copy(keyRemove = null)

    /** "Replace it instead": closes the question and asks the section to fill in the name. */
    fun replace(ui: SettingsUi): SettingsUi {
        val q = ui.keyRemove ?: return ui
        return if (q.busy) ui else ui.copy(keyRemove = null, keyReplace = q.name)
    }

    fun begin(ui: SettingsUi): SettingsUi = ui.keyRemove?.let { ui.copy(keyRemove = it.copy(busy = true)) } ?: ui

    /** The removal ended: the question closes, the section says what happened and reads the keys again. */
    fun finished(ui: SettingsUi, line: String, ok: Boolean): SettingsUi = ui.copy(keyRemove = null, keysLine = line, keysLineBad = !ok, keysVersion = ui.keysVersion + 1)

    fun replaceHandled(ui: SettingsUi): SettingsUi = ui.copy(keyReplace = null)
}

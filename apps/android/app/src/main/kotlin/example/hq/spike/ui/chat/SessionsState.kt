package example.hq.spike.ui.chat

import example.hq.spike.data.WatchedSession

/** The agents the open chat watches, for the header button and the sessions sheet. */
data class SessionsUi(
    val open: Boolean = false,
    val sessions: List<WatchedSession> = emptyList(),
    /** Sessions with a Drive or stop request in flight; their rows ignore taps and a refresh keeps their local state. */
    val busy: Set<String> = emptySet(),
) {
    val count: Int get() = sessions.size
    val waiting: Int get() = sessions.count { it.needsAnswer }

    /** Agents HQ answers for right now: running with Drive on. */
    val driving: List<WatchedSession> get() = sessions.filter { it.running && it.drive }
}

/** The rules for [SessionsUi], pure so the optimistic switch and its rollback are tested without a phone. */
object SessionsReducer {
    /** A refresh replaces the list but keeps the local value of a row whose request is still in flight. */
    fun loaded(ui: SessionsUi, fresh: List<WatchedSession>): SessionsUi {
        val local = ui.sessions.associateBy { it.id }
        val merged = fresh.map { s -> if (s.id in ui.busy) local[s.id] ?: s else s }
        return ui.copy(sessions = merged, busy = ui.busy.filter { id -> fresh.any { it.id == id } }.toSet())
    }

    /** Flips the switch at once and marks the row busy. A row already busy or not listed is left alone. */
    fun driveRequested(ui: SessionsUi, id: String, drive: Boolean): SessionsUi {
        if (id in ui.busy || ui.sessions.none { it.id == id }) return ui
        return ui.copy(sessions = ui.sessions.map { if (it.id == id) it.copy(drive = drive) else it }, busy = ui.busy + id)
    }

    /** HQ's saved row replaces the local one. */
    fun driveSaved(ui: SessionsUi, saved: WatchedSession): SessionsUi =
        ui.copy(sessions = ui.sessions.map { if (it.id == saved.id) saved else it }, busy = ui.busy - saved.id)

    /** The request failed: the switch goes back to what it was. */
    fun driveFailed(ui: SessionsUi, id: String, was: Boolean): SessionsUi =
        ui.copy(sessions = ui.sessions.map { if (it.id == id) it.copy(drive = was) else it }, busy = ui.busy - id)

    fun stopRequested(ui: SessionsUi, id: String): SessionsUi =
        if (id in ui.busy || ui.sessions.none { it.id == id }) ui else ui.copy(busy = ui.busy + id)

    fun stopped(ui: SessionsUi, id: String): SessionsUi = ui.copy(sessions = ui.sessions.filterNot { it.id == id }, busy = ui.busy - id)

    fun stopFailed(ui: SessionsUi, id: String): SessionsUi = ui.copy(busy = ui.busy - id)

    /** The name a row and its screen reader label use: the label, else the harness, else a plain word. */
    fun name(s: WatchedSession, fallback: String): String = s.label.ifBlank { s.harness }.ifBlank { fallback }
}

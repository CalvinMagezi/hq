package example.hq.spike.ui.firstrun

import android.content.Context

/** The screens of the first-run flow. [Restore] hangs off [Welcome] and ends at [Start]. */
enum class Step { Welcome, Model, Chat, Start, Restore }

/** The pure rules: which screen shows on launch, and where each button leads. */
object FirstRunLogic {
    /**
     * The screen to show now, or null when the flow is over.
     * A finished flow stays finished. A saved step is resumed even after a kill, and wins over a stored key (the key
     * is sealed at step 1, so a later step has one already). With nothing saved, a stored model key or an app that
     * had already heard HQ answer before this flow existed (an upgrade, or a development build set up with adb)
     * means there is nothing to ask. Only a clean install starts at [Step.Welcome].
     */
    fun decide(done: Boolean, saved: Step?, hasModelKey: Boolean, heardHqBefore: Boolean): Step? = when {
        done -> null
        saved != null -> saved
        hasModelKey || heardHqBefore -> null
        else -> Step.Welcome
    }

    fun next(step: Step): Step = when (step) {
        Step.Welcome -> Step.Model
        Step.Model -> Step.Chat
        Step.Chat -> Step.Start
        Step.Start -> Step.Start
        Step.Restore -> Step.Start
    }

    fun back(step: Step): Step = when (step) {
        Step.Welcome -> Step.Welcome
        Step.Model -> Step.Welcome
        Step.Chat -> Step.Model
        Step.Start -> Step.Chat
        Step.Restore -> Step.Welcome
    }
}

/** The one thing kept across a kill: which step the user was on, and whether the flow is finished. Never a credential. */
class FirstRunStore(context: Context) {
    private val prefs = context.applicationContext.getSharedPreferences(FILE, Context.MODE_PRIVATE)

    fun done(): Boolean = prefs.getBoolean(DONE, false)
    fun step(): Step? = prefs.getString(STEP, null)?.let { name -> Step.entries.firstOrNull { it.name == name } }
    fun save(step: Step) { prefs.edit().putString(STEP, step.name).apply() }
    fun finish() { prefs.edit().putBoolean(DONE, true).remove(STEP).apply() }

    private companion object {
        const val FILE = "firstrun"
        const val STEP = "step"
        const val DONE = "done"
    }
}

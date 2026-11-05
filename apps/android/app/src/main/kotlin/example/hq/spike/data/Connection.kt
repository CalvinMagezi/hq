package example.hq.spike.data

import androidx.annotation.StringRes
import example.hq.spike.R

/**
 * The one connection state every place reads (rules-shared.md 6, resolutions.md D-5). Wave 0 builds the
 * model and its words; the sheet that shows it is ui/connection (wave 2), and HqServices moves onto it in
 * wave 4 (mobile/wave-4/01-offline.md).
 */
sealed interface Connection {
    data object Connected : Connection
    data class Starting(val step: Int, val seconds: Long, val usualSeconds: Long?) : Connection
    data class Restarting(val seconds: Long) : Connection
    data class Stopped(val sinceMs: Long, val step: Int?) : Connection
    data class Reconnecting(val sinceMs: Long) : Connection
    data class ServerUnreachable(val sinceMs: Long) : Connection
    data object ServerSignedOut : Connection
    data class Offline(val sinceMs: Long) : Connection
    data class AiNotAnswering(val service: String?, val atMs: Long) : Connection
}

/** What the phone can see, from which [decide] picks one state. */
data class ConnectionInputs(
    val nowMs: Long,
    val processAlive: Boolean,
    val restarting: Boolean,
    /** 0-based start step, or null when HQ answered. */
    val startStep: Int?,
    val startedAtMs: Long?,
    /** When the start step last moved. */
    val stepMovedAtMs: Long?,
    val usualStartMs: Long?,
    val serverSignedOut: Boolean,
    val serverUnreachableSinceMs: Long?,
    val socketDownSinceMs: Long?,
    val offlineSinceMs: Long?,
    val aiFailingService: String?,
    val aiFailingAtMs: Long?,
)

object ConnectionRules {
    /** HQ counts as stopped when its start step has not moved for the longer of this and twice its usual start (A7). */
    const val STUCK_FLOOR_MS = 60_000L
    /** "You're offline" only after this long with no validated network (wave-4/01-offline.md rule 7). */
    const val OFFLINE_AFTER_MS = 3_000L
    /** The AI state clears by itself after this long without a new failure (rule 8). */
    const val AI_STATE_MS = 10 * 60_000L

    /** Stopped: the process ended, or its start has not moved for max(60 s, 2 x the usual start). No fixed timer. */
    fun stuck(i: ConnectionInputs): Boolean {
        if (!i.processAlive) return true
        val moved = i.stepMovedAtMs ?: return false
        val limit = maxOf(STUCK_FLOOR_MS, 2 * (i.usualStartMs ?: 0L))
        return i.nowMs - moved >= limit
    }

    /** The first true row wins (rule 1). */
    fun decide(i: ConnectionInputs): Connection {
        val since = { at: Long? -> at ?: i.nowMs }
        return when {
            stuck(i) -> Connection.Stopped(since(i.stepMovedAtMs), i.startStep)
            i.restarting -> Connection.Restarting(seconds(i.nowMs, i.startedAtMs))
            i.startStep != null -> Connection.Starting(i.startStep, seconds(i.nowMs, i.startedAtMs), i.usualStartMs?.div(1000))
            i.serverSignedOut -> Connection.ServerSignedOut
            i.serverUnreachableSinceMs != null -> Connection.ServerUnreachable(i.serverUnreachableSinceMs)
            i.socketDownSinceMs != null -> Connection.Reconnecting(i.socketDownSinceMs)
            i.offlineSinceMs != null && i.nowMs - i.offlineSinceMs >= OFFLINE_AFTER_MS -> Connection.Offline(i.offlineSinceMs)
            i.aiFailingAtMs != null && i.nowMs - i.aiFailingAtMs < AI_STATE_MS -> Connection.AiNotAnswering(i.aiFailingService, i.aiFailingAtMs)
            else -> Connection.Connected
        }
    }

    private fun seconds(nowMs: Long, fromMs: Long?): Long = if (fromMs == null) 0 else maxOf(0L, (nowMs - fromMs) / 1000)
}

/** The dot beside the words: grey when all is well, blue while it works on it, amber when it needs you. */
enum class ConnectionTone { Quiet, Working, Warn }

/** The status-line words and tone for each state, from the one catalogue (apps/strings/en.yaml). */
object ConnectionWords {
    data class Words(@StringRes val res: Int, val args: List<Any> = emptyList(), val tone: ConnectionTone)

    fun of(c: Connection): Words = when (c) {
        Connection.Connected -> Words(R.string.state_connected, tone = ConnectionTone.Quiet)
        is Connection.Starting -> Words(R.string.state_starting, listOf(c.seconds.toString()), ConnectionTone.Working)
        is Connection.Restarting -> Words(R.string.state_hub_restarting, tone = ConnectionTone.Working)
        is Connection.Stopped -> Words(R.string.state_hub_stopped, tone = ConnectionTone.Warn)
        is Connection.Reconnecting -> Words(R.string.state_reconnecting, tone = ConnectionTone.Working)
        is Connection.ServerUnreachable -> Words(R.string.state_server_unreachable, tone = ConnectionTone.Warn)
        Connection.ServerSignedOut -> Words(R.string.state_server_signed_out, tone = ConnectionTone.Warn)
        is Connection.Offline -> Words(R.string.state_offline, tone = ConnectionTone.Warn)
        is Connection.AiNotAnswering -> Words(R.string.state_ai_down, tone = ConnectionTone.Warn)
    }

    /** The reason sentence of a failure line for a state, or null when nothing is wrong (A2). */
    @StringRes fun reason(c: Connection): Int? = when (c) {
        Connection.Connected -> null
        is Connection.Starting -> R.string.reason_starting
        is Connection.Restarting -> R.string.reason_restarting
        is Connection.Stopped -> R.string.reason_stopped
        is Connection.Reconnecting, is Connection.ServerUnreachable -> R.string.reason_server_unreachable
        Connection.ServerSignedOut -> R.string.reason_server_signed_out
        is Connection.Offline -> R.string.reason_offline
        is Connection.AiNotAnswering -> R.string.reason_ai_down
    }

    /** The outcome sentence: it depends only on whether the action waits and retries by itself (A2). */
    @StringRes fun outcome(c: Connection, retriesItself: Boolean): Int? = when (c) {
        Connection.Connected -> null
        is Connection.Starting, is Connection.Restarting -> if (retriesItself) R.string.outcome_retries_itself else R.string.outcome_try_moment
        is Connection.Stopped -> if (retriesItself) R.string.outcome_retries_when_back else R.string.outcome_start_hq
        is Connection.Reconnecting, is Connection.ServerUnreachable -> if (retriesItself) R.string.outcome_retries_when_back else R.string.outcome_try_when_back
        Connection.ServerSignedOut -> R.string.outcome_sign_in_again
        is Connection.Offline -> if (retriesItself) R.string.outcome_retries_when_online else R.string.outcome_try_when_online
        is Connection.AiNotAnswering -> R.string.outcome_hq_fine
    }
}

package example.hq.spike.ui.shell

import android.content.Context
import android.os.Build
import android.view.accessibility.AccessibilityManager
import example.hq.spike.R
import example.hq.spike.data.HqClient
import example.hq.spike.data.HqUnavailable
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts
import org.json.JSONObject

/** What Undo puts back: each kind has its own restore route on HQ. */
enum class UndoKind { Thread, Notification, Task, Budgets }

/** How a restore ended. [TooLate] is HQ's 409: past 10 minutes, or the item changed on another device. */
enum class UndoResult { Done, TooLate, Unreachable }

/** The restore calls behind one seam so tests can fake HQ. */
interface UndoRestore {
    suspend fun unarchiveThread(id: String)
    suspend fun restoreNotification(id: String)
    suspend fun restoreTask(id: String)
    suspend fun restoreBudgets(limits: JSONObject)
}

class ClientRestore(private val client: HqClient) : UndoRestore {
    override suspend fun unarchiveThread(id: String) = client.unarchiveThread(id)
    override suspend fun restoreNotification(id: String) = client.restoreNotification(id)
    override suspend fun restoreTask(id: String) = client.restoreTask(id)
    override suspend fun restoreBudgets(limits: JSONObject) = client.restoreBudgets(limits)
}

object Undo {
    /** Android 8 and 9 have no accessibility timeout setting, so Undo stays 10 s there. */
    const val LEGACY_MS = 10_000L
    private const val CONTENT_MS = 5_000

    /** The phone's own timeout for 5 s of content (Android 10+), never shorter than 5 s. */
    fun millis(sdk: Int, recommended: (Int) -> Int): Long =
        if (sdk < Build.VERSION_CODES.Q) LEGACY_MS else recommended(CONTENT_MS).toLong().coerceAtLeast(CONTENT_MS.toLong())

    fun millis(context: Context): Long = millis(Build.VERSION.SDK_INT) { base ->
        val am = context.getSystemService(Context.ACCESSIBILITY_SERVICE) as? AccessibilityManager
        if (am == null || Build.VERSION.SDK_INT < Build.VERSION_CODES.Q) base
        else am.getRecommendedTimeoutMillis(base, AccessibilityManager.FLAG_CONTENT_TEXT or AccessibilityManager.FLAG_CONTENT_CONTROLS)
    }

    /** Sends the restore for [kind]; a 409 is [UndoResult.TooLate], any other failure [UndoResult.Unreachable]. */
    suspend fun restore(restore: UndoRestore, kind: UndoKind, id: String, limits: JSONObject? = null): UndoResult = try {
        when (kind) {
            UndoKind.Thread -> restore.unarchiveThread(id)
            UndoKind.Notification -> restore.restoreNotification(id)
            UndoKind.Task -> restore.restoreTask(id)
            UndoKind.Budgets -> restore.restoreBudgets(requireNotNull(limits))
        }
        UndoResult.Done
    } catch (e: HqUnavailable) {
        if (e.status == 409) UndoResult.TooLate else UndoResult.Unreachable
    }

    /** The line a refused restore shows, or null when it worked or HQ did not answer (that one is held in the outbox). */
    fun failLine(context: Context, result: UndoResult): String? =
        if (result == UndoResult.TooLate) context.getString(R.string.fail_undo_too_late) else null
}

/** The one Undo snackbar: what happened, then Undo. The owner of [text] decides when it closes (see [Undo.millis]). */
@Composable
fun UndoSnackbar(text: String, onUndo: () -> Unit, modifier: Modifier = Modifier) {
    Row(
        modifier.padding(8.dp).fillMaxWidth().heightIn(min = 48.dp).background(Mts.Ink, RoundedCornerShape(6.dp)).padding(start = 16.dp).semantics { liveRegion = LiveRegionMode.Polite },
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(text, style = HqType.BodySm, color = Mts.Surface100, modifier = Modifier.weight(1f))
        Box(Modifier.heightIn(min = 48.dp).clickable(role = Role.Button, onClick = onUndo).padding(horizontal = 16.dp).testTag("undo"), contentAlignment = Alignment.Center) {
            Text(stringResource(R.string.undo), style = HqType.BodySmBold, color = Mts.UndoBlue)
        }
    }
}

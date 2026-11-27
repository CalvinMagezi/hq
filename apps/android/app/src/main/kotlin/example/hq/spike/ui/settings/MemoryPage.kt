package example.hq.spike.ui.settings

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Switch
import androidx.compose.material3.SwitchDefaults
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import example.hq.spike.R
import example.hq.spike.data.HqServices
import example.hq.spike.data.HqUnavailable
import example.hq.spike.ui.shell.ConfirmCopy
import example.hq.spike.ui.shell.ConfirmDialog
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts
import kotlinx.coroutines.launch
import org.json.JSONObject

/** One remembered note, as `/api/memory` lists it. */
internal data class MemoryNote(
    val id: Long,
    val summary: String,
    val day: String,
    val pinned: Boolean,
    val status: String,
    val origin: String,
    val used: Int,
)

internal data class MemoryCounts(val total: Int, val active: Int, val review: Int, val outdated: Int, val archived: Int)

internal data class MemoryList(val notes: List<MemoryNote>, val counts: MemoryCounts, val recall: String)

/** What the page knows how to read from HQ's answer. Anything missing reads as empty rather than failing. */
internal object MemoryFacts {
    const val PAGE_SIZE = 50

    fun parse(o: JSONObject): MemoryList {
        val arr = o.optJSONArray("items")
        val notes = if (arr == null) emptyList() else (0 until arr.length()).map { i ->
            val n = arr.getJSONObject(i)
            MemoryNote(
                id = n.optLong("id"),
                summary = n.optString("summary"),
                day = n.optString("created_at").take(10),
                pinned = n.optBoolean("pinned"),
                status = n.optString("status"),
                origin = n.optString("origin"),
                used = n.optInt("used"),
            )
        }
        val c = o.optJSONObject("counts")
        val counts = MemoryCounts(
            total = c?.optInt("total") ?: notes.size,
            active = c?.optInt("active") ?: notes.size,
            review = c?.optInt("review") ?: 0,
            outdated = c?.optInt("out_of_date") ?: 0,
            archived = c?.optInt("archived") ?: 0,
        )
        return MemoryList(notes, counts, o.optString("recall"))
    }

    /** Whether the note is one HQ holds back until the owner has looked: a held-back row, or one from a shared channel. */
    fun heldBack(n: MemoryNote): Boolean = n.status == "review" || n.origin != "owner"

    /** The plain-words line under a note HQ treats differently, as the web page shows it; null for an ordinary note. */
    fun hintFor(n: MemoryNote): Int? = when {
        n.origin != "owner" -> R.string.memory_hint_shared
        n.status == "review" -> R.string.memory_hint_review
        n.status == "archived" -> R.string.memory_hint_archived
        else -> null
    }

    /** The standing action a note offers, as the web page does: keep a held-back one, use an old one again, or mark a live one out of date. */
    fun standing(n: MemoryNote): Pair<Int, String> = when {
        heldBack(n) -> R.string.memory_keep to "active"
        n.status == "superseded" || n.status == "archived" -> R.string.memory_use_again to "active"
        else -> R.string.memory_mark_outdated to "outdated"
    }
}

private sealed interface MemRead {
    data object Loading : MemRead
    data object Failed : MemRead
    data class Got(val list: MemoryList) : MemRead
}

/**
 * What HQ remembers: the notes it saved from the owner's chats, newest first, each with Pin, Keep and Forget. Editing and searching
 * are on the web page and in `hq memory`; the phone shows and corrects what it can without a keyboard.
 */
@Composable
internal fun MemoryPage() {
    val context = LocalContext.current
    val client = remember { HqServices.get(context).client }
    val scope = rememberCoroutineScope()
    var read by remember { mutableStateOf<MemRead>(MemRead.Loading) }
    var learning by remember { mutableStateOf<Boolean?>(null) }
    var openId by rememberSaveable { mutableStateOf(-1L) }
    var confirmId by rememberSaveable { mutableStateOf(-1L) }
    var busy by remember { mutableStateOf(false) }
    var failed by remember { mutableStateOf(false) }
    var forgotten by remember { mutableStateOf(false) }

    suspend fun load() {
        read = try {
            MemRead.Got(MemoryFacts.parse(client.json("/api/memory?limit=${MemoryFacts.PAGE_SIZE}")))
        } catch (e: HqUnavailable) {
            MemRead.Failed
        }
        // Older HQ builds have no such pref; the switch then stays off the table.
        learning = try {
            client.json("/api/prefs").let { if (it.has("memory.learning")) it.optBoolean("memory.learning", true) else null }
        } catch (e: HqUnavailable) {
            null
        }
    }
    LaunchedEffect(Unit) { load() }

    fun change(call: suspend () -> Unit) {
        scope.launch {
            busy = true
            failed = false
            try {
                call()
                load()
            } catch (e: HqUnavailable) {
                failed = true
            }
            busy = false
        }
    }

    Page {
        Text(stringResource(R.string.memory_intro), style = HqType.BodySm, color = Mts.Ink, modifier = Modifier.padding(start = 16.dp, top = 4.dp, end = 16.dp))
        when (val r = read) {
            MemRead.Loading -> Text(stringResource(R.string.settings_loading), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(start = 16.dp, top = 12.dp, end = 16.dp))
            MemRead.Failed -> {
                Text(stringResource(R.string.memory_load_failed), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(start = 16.dp, top = 12.dp, end = 16.dp))
                Group {
                    Column(Modifier.fillMaxWidth().heightIn(min = 56.dp).clickable(role = Role.Button) { read = MemRead.Loading; scope.launch { load() } }.padding(horizontal = 16.dp, vertical = 8.dp), verticalArrangement = Arrangement.Center) {
                        Text(stringResource(R.string.settings_try_again), style = HqType.BodyBold, color = Mts.Ink)
                    }
                }
            }
            is MemRead.Got -> {
                val c = r.list.counts
                Text(stringResource(R.string.memory_counts, c.total, c.active, c.review, c.outdated, c.archived), style = HqType.BodySm, color = Mts.Ink, modifier = Modifier.padding(start = 16.dp, top = 8.dp, end = 16.dp))
                val mode = when (r.list.recall) {
                    "shadow" -> R.string.memory_mode_shadow
                    "query" -> R.string.memory_mode_query
                    "recent" -> R.string.memory_mode_recent
                    "off" -> R.string.memory_mode_off
                    else -> null
                }
                if (mode != null) Text(stringResource(mode), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(start = 16.dp, top = 4.dp, end = 16.dp))
                learning?.let { on ->
                    Group {
                        LearningRow(on, !busy) { next -> change { client.putJson("/api/prefs", JSONObject().put("key", "memory.learning").put("value", next)) } }
                    }
                }
                if (failed) Text(stringResource(R.string.memory_action_failed), style = HqType.BodySm, color = Mts.Amber, modifier = Modifier.padding(start = 16.dp, top = 12.dp, end = 16.dp))
                if (forgotten) {
                    Text(stringResource(R.string.memory_forgotten), style = HqType.BodySm, color = Mts.Ink, modifier = Modifier.padding(start = 16.dp, top = 12.dp, end = 16.dp))
                    Text(stringResource(R.string.memory_still_there), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(start = 16.dp, top = 4.dp, end = 16.dp))
                    Text(stringResource(R.string.memory_still_chat), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(start = 16.dp, end = 16.dp))
                    Text(stringResource(R.string.memory_still_backups), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(start = 16.dp, end = 16.dp))
                }
                if (r.list.notes.isEmpty()) {
                    Text(stringResource(R.string.memory_empty), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(start = 16.dp, top = 12.dp, end = 16.dp))
                } else {
                    Group {
                        r.list.notes.forEachIndexed { i, note ->
                            if (i > 0) Divider()
                            NoteRow(note, open = openId == note.id, busy = busy,
                                onOpen = { openId = if (openId == note.id) -1L else note.id },
                                onPin = { change { client.postJson("/api/memory/${note.id}/pin", JSONObject().put("pinned", !note.pinned)) } },
                                onStanding = { change { client.postJson("/api/memory/${note.id}/standing", JSONObject().put("standing", MemoryFacts.standing(note).second)) } },
                                onForget = { confirmId = note.id })
                        }
                    }
                }
            }
        }
    }
    if (confirmId >= 0) {
        val id = confirmId
        ConfirmDialog(
            ConfirmCopy.forgetMemory(context), { confirmId = -1L },
            {
                scope.launch {
                    busy = true
                    failed = false
                    forgotten = false
                    try {
                        client.postJson("/api/memory/$id/forget")
                        forgotten = true
                        openId = -1L
                        confirmId = -1L
                        load()
                    } catch (e: HqUnavailable) {
                        failed = true
                        confirmId = -1L
                    }
                    busy = false
                }
            },
            busy = busy,
        )
    }
}

@Composable
private fun LearningRow(on: Boolean, enabled: Boolean, onToggle: (Boolean) -> Unit) {
    Row(
        Modifier.fillMaxWidth().heightIn(min = 64.dp).padding(horizontal = 16.dp, vertical = 8.dp),
        horizontalArrangement = Arrangement.spacedBy(16.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f)) {
            Text(stringResource(R.string.memory_learning_item), style = HqType.BodyBold, color = Mts.Ink)
            Text(stringResource(if (on) R.string.memory_learning_on else R.string.memory_learning_paused), style = HqType.BodySm, color = Mts.InkMuted)
        }
        Switch(
            checked = on,
            onCheckedChange = if (enabled) onToggle else null,
            colors = SwitchDefaults.colors(
                checkedTrackColor = Mts.BrandGreen,
                checkedThumbColor = Mts.OnGreen,
                checkedBorderColor = Mts.BrandGreen,
                uncheckedTrackColor = Mts.Surface200,
                uncheckedThumbColor = Mts.Line,
                uncheckedBorderColor = Mts.Line,
            ),
        )
    }
}

@Composable
private fun NoteRow(
    note: MemoryNote,
    open: Boolean,
    busy: Boolean,
    onOpen: () -> Unit,
    onPin: () -> Unit,
    onStanding: () -> Unit,
    onForget: () -> Unit,
) {
    val badges = buildList {
        if (note.pinned) add(stringResource(R.string.memory_badge_pinned))
        if (note.status == "review") add(stringResource(R.string.memory_badge_review))
        if (note.status == "superseded") add(stringResource(R.string.memory_badge_outdated))
        if (note.status == "archived") add(stringResource(R.string.memory_badge_archived))
        if (note.origin != "owner") add(stringResource(R.string.memory_badge_shared))
    }
    val used = if (note.used > 0) pluralStringResource(R.plurals.memory_used, note.used, note.used) else null
    val meta = (listOf(note.day) + badges + listOfNotNull(used)).filter { it.isNotBlank() }.joinToString(" · ")
    Column(Modifier.fillMaxWidth().clickable(role = Role.Button) { onOpen() }.padding(horizontal = 16.dp, vertical = 8.dp)) {
        Text(note.summary, style = HqType.BodyBold, color = Mts.Ink)
        Text(meta, style = HqType.BodySm, color = Mts.InkMuted)
        MemoryFacts.hintFor(note)?.let { Text(stringResource(it), style = HqType.BodySm, color = Mts.InkMuted) }
        if (open) {
            val standing = stringResource(MemoryFacts.standing(note).first)
            Row(Modifier.fillMaxWidth().padding(top = 8.dp), horizontalArrangement = Arrangement.spacedBy(16.dp)) {
                Action(stringResource(if (note.pinned) R.string.memory_unpin else R.string.memory_pin), !busy, onPin)
                Action(standing, !busy, onStanding)
                Action(stringResource(R.string.memory_forget), !busy, onForget)
            }
        }
    }
}

@Composable
private fun Action(text: String, enabled: Boolean, onClick: () -> Unit) {
    Text(
        text,
        style = HqType.BodyBold,
        color = if (enabled) Mts.Ink else Mts.InkMuted,
        modifier = Modifier.heightIn(min = 48.dp).clickable(enabled = enabled, role = Role.Button) { onClick() }.padding(vertical = 12.dp),
    )
}

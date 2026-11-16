package example.hq.spike.ui.chat

import androidx.annotation.StringRes
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import example.hq.spike.R
import example.hq.spike.data.ToolStep
import example.hq.spike.ui.shell.AttentionGate
import example.hq.spike.ui.shell.AttentionKind
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts

enum class ReplyToolKind { NoteSaved, TaskMade, SearchOrRead, None }

/**
 * Client-side follow-up chips shown after a reply finishes streaming.
 * Proposal: client-side static table until HQ sends a follow-up hint on the reply.
 */
object FollowUpChips {
    const val MAX = 3
    const val LABEL_MAX = 40

    // Proposal: static table per tool kind until HQ returns server-side follow-up hints.
    val TABLE: Map<ReplyToolKind, List<Int>> = mapOf(
        ReplyToolKind.NoteSaved to listOf(
            R.string.chat_chip_note_1,
            R.string.chat_chip_note_2,
            R.string.chat_chip_note_3,
        ),
        ReplyToolKind.TaskMade to listOf(
            R.string.chat_chip_task_1,
            R.string.chat_chip_task_2,
            R.string.chat_chip_task_3,
        ),
        ReplyToolKind.SearchOrRead to listOf(
            R.string.chat_chip_search_1,
            R.string.chat_chip_search_2,
            R.string.chat_chip_search_3,
        ),
        ReplyToolKind.None to listOf(
            R.string.chat_chip_neutral_1,
            R.string.chat_chip_neutral_2,
            R.string.chat_chip_neutral_3,
        ),
    )

    private val NOTE_SAVED_TOOLS = setOf(
        "vault_write_note", "vault_append_note", "vault_patch_section",
        "vault_frontmatter_update", "vault_import", "vault_move",
    )

    private val TASK_MADE_TOOLS = setOf(
        "task_create", "task_create_from_note", "task_create_many",
        "task_update", "task_update_many",
    )

    private val SEARCH_OR_READ_TOOLS = setOf(
        "web_search", "web_fetch", "vault_search", "vault_find", "vault_find_similar",
        "vault_read", "vault_read_section", "vault_outline", "vault_batch_read",
        "vault_list", "vault_tags", "vault_links", "vault_backlinks", "vault_context",
        "vault_inventory", "context_packet", "folder_list", "task_list", "task_get",
        "task_next", "session_search", "history_restore", "read_file", "grep",
        "find_files", "list_dir",
    )

    fun classify(tools: List<ToolStep>): ReplyToolKind {
        val names = tools.map { it.name }
        return when {
            names.any { it in NOTE_SAVED_TOOLS } -> ReplyToolKind.NoteSaved
            names.any { it in TASK_MADE_TOOLS } -> ReplyToolKind.TaskMade
            names.any { it in SEARCH_OR_READ_TOOLS } -> ReplyToolKind.SearchOrRead
            else -> ReplyToolKind.None
        }
    }

    fun forTools(tools: List<ToolStep>): List<Int> =
        TABLE[classify(tools)].orEmpty().take(MAX)

    /** Returns up to [MAX] chip string ids when connected, a turn finished in this session, and no blocker/typing exists. */
    fun forState(state: ChatUiState, hasApproval: Boolean, hasProblemBar: Boolean): List<Int> {
        if (!state.connected || hasApproval || hasProblemBar) return emptyList()
        if (state.offered != null || state.offerResult != null) return emptyList()
        if (state.live != null || state.failed || state.draft.isNotEmpty()) return emptyList()
        val last = state.messages.lastOrNull() ?: return emptyList()
        if (last.speaker != Speaker.Reply || last.sending || last.notSent) return emptyList()
        if (last.id != state.finishedReplyId || last.id == state.dismissedFollowUpId) return emptyList()
        return forTools(last.tools)
    }

    /** Records dismissal when the user types so clearing the draft back to empty keeps chips hidden. */
    fun onDraftChanged(state: ChatUiState, text: String): ChatUiState {
        val dismissed = if (text.isNotEmpty()) {
            state.messages.lastOrNull()?.takeIf { it.speaker == Speaker.Reply }?.id ?: state.dismissedFollowUpId
        } else {
            state.dismissedFollowUpId
        }
        return state.copy(draft = text, dismissedFollowUpId = dismissed)
    }
}

private val ChipShape = RoundedCornerShape(20.dp)

@OptIn(ExperimentalLayoutApi::class)
@Composable
fun FollowUpChipsSlot(@StringRes chipIds: List<Int>, replyKey: String, onPick: (String) -> Unit) {
    if (chipIds.isEmpty()) return
    AttentionGate(AttentionKind.Pills, replyKey) {
        FlowRow(
            Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            chipIds.forEach { resId ->
                val text = stringResource(resId)
                Row(
                    Modifier.heightIn(min = 48.dp).clip(ChipShape).background(Mts.Surface100)
                        .border(1.dp, Mts.Line, ChipShape).clickable(role = Role.Button) { onPick(text) }
                        .padding(horizontal = 14.dp, vertical = 10.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(text, style = HqType.BodySm, color = Mts.Ink)
                }
            }
        }
    }
}

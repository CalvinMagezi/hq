package example.hq.spike.capture

import android.content.Intent
import android.os.Bundle
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import example.hq.spike.ShellActivity
import example.hq.spike.data.HqServices
import example.hq.spike.ui.shell.HqIcons
import example.hq.spike.ui.shell.Links
import example.hq.spike.ui.theme.HqTheme
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts
import java.time.LocalDate
import java.util.UUID
import kotlinx.coroutines.runBlocking

/**
 * Save a thought, a link or a task from anywhere in Android in one step: a small sheet over the app the owner came from.
 * A save goes to app storage first, the sheet closes at once, and the send happens in the background.
 */
class CaptureActivity : ComponentActivity() {
    private val inbox = "Notebooks/Inbox"

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge(statusBarStyle = SystemBarStyle.dark(android.graphics.Color.TRANSPARENT), navigationBarStyle = SystemBarStyle.dark(android.graphics.Color.TRANSPARENT))
        val subject = intent.getStringExtra(Intent.EXTRA_SUBJECT)
        val text = intent.getCharSequenceExtra(Intent.EXTRA_TEXT)?.toString()
        val shared = intent.action == Intent.ACTION_SEND
        val asked = intent.getStringExtra(EXTRA_MODE)
        val start = if (shared && asked == null) CapturePrefs.startMode(null, CapturePrefs.shareOpens(this), CapturePrefs.lastMode(this)) else CapturePrefs.modeOf(asked) ?: CaptureMode.Note
        val client = HqServices.get(applicationContext).client
        setContent {
            HqTheme {
                val folders by produceFolders { runCatching { client.folders() }.getOrDefault(emptyList()) }
                CaptureSheet(start, subject, text, folders, onClose = ::finish, onSave = ::save)
            }
        }
    }

    /** The folders for "Save in", Inbox first and Notebooks only; empty while HQ does not answer. */
    @Composable
    private fun produceFolders(load: suspend () -> List<String>) = androidx.compose.runtime.produceState(listOf(inbox)) {
        val all = load().filter { it.startsWith("Notebooks/") }
        if (all.isNotEmpty()) value = (listOf(inbox) + all).distinct()
    }

    private fun save(draft: CaptureDraft, andOpen: Boolean) {
        CapturePrefs.rememberMode(this, draft.mode)
        if (draft.mode == CaptureMode.Chat) {
            startActivity(Intent(this, ShellActivity::class.java).putExtra(Links.EXTRA, "hq://chat").putExtra(EXTRA_DRAFT, CaptureWords.chatDraft(draft.title, draft.body)).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP))
            finish()
            return
        }
        val id = "capture-" + UUID.randomUUID()
        val q = if (draft.mode == CaptureMode.Note) {
            val body = CaptureWords.noteBody(draft.folder, draft.title, draft.body)
            Queued(id, Queued.KIND_NOTE, body.getString("title"), body, System.currentTimeMillis())
        } else {
            Queued(id, Queued.KIND_TASK, draft.title.trim(), CaptureWords.taskBody(id, draft.title, draft.body, draft.priority, draft.due, CaptureWords.assignees(draft.assignTo)), System.currentTimeMillis())
        }
        val box = Outbox.of(this)
        box.add(q)
        val app = applicationContext
        if (andOpen && q.isNote) {
            // The note's path only comes back from HQ, so opening it waits for the send; if HQ is down it opens later from the outbox.
            Thread {
                val result = runBlocking { OutboxSender.send(HqServices.get(app).client, q) }
                runOnUiThread {
                    if (result is Sent.Done && result.path != null) {
                        box.remove(q.id)
                        startActivity(Intent(this, ShellActivity::class.java).putExtra(Links.EXTRA, "hq://notes/" + result.path).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP))
                    } else Toast.makeText(app, "Saved. It is sent when HQ is back.", Toast.LENGTH_LONG).show()
                    finish()
                }
            }.start()
            return
        }
        Toast.makeText(app, "Saved", Toast.LENGTH_SHORT).show()
        Thread { OutboxSender.flushNow(app) }.start()
        finish()
    }

    companion object {
        const val EXTRA_MODE = "mode"
        const val EXTRA_DRAFT = "draft"
    }
}

/** What is typed in the sheet. */
data class CaptureDraft(
    val mode: CaptureMode, val title: String, val body: String, val folder: String,
    val priority: String? = null, val due: String? = null, val assignTo: String = "",
)

private const val SCRIM_ALPHA = 0.62f
private val Priorities = listOf<Pair<String?, String>>(null to "None", "low" to "Low", "normal" to "Normal", "high" to "High", "urgent" to "Urgent")

@Composable
private fun CaptureSheet(start: CaptureMode, subject: String?, text: String?, folders: List<String>, onClose: () -> Unit, onSave: (CaptureDraft, Boolean) -> Unit) {
    var mode by remember { mutableStateOf(start) }
    var title by remember { mutableStateOf(CaptureWords.titleFor(subject, text)) }
    // A link goes in the note's text; when the app shared only a link there is nothing else to write.
    var body by remember { mutableStateOf(text.orEmpty()) }
    var folder by remember { mutableStateOf("Notebooks/Inbox") }
    var pickFolder by remember { mutableStateOf(false) }
    var more by remember { mutableStateOf(false) }
    var priority by remember { mutableStateOf<String?>(null) }
    var due by remember { mutableStateOf<String?>(null) }
    var assignTo by remember { mutableStateOf("") }
    LaunchedEffect(folders) { if (folder !in folders && folders.isNotEmpty()) folder = folders.first() }
    val preview = remember(subject, text) { CaptureWords.preview(subject, text) }
    val shape = RoundedCornerShape(topStart = 28.dp, topEnd = 28.dp)
    val canSave = when (mode) { CaptureMode.Note -> title.isNotBlank() || body.isNotBlank(); CaptureMode.Task -> title.isNotBlank(); CaptureMode.Chat -> title.isNotBlank() || body.isNotBlank() }
    Box(Modifier.fillMaxSize()) {
        Box(Modifier.fillMaxSize().background(Color.Black.copy(alpha = SCRIM_ALPHA)).clickable(interactionSource = remember { MutableInteractionSource() }, indication = null, onClick = onClose))
        Column(
            Modifier.align(Alignment.BottomCenter).fillMaxWidth().clip(shape).background(Mts.Surface100).clickable(interactionSource = remember { MutableInteractionSource() }, indication = null) {}
                .imePadding().navigationBarsPadding().padding(start = 16.dp, end = 16.dp, top = 12.dp, bottom = 16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Box(Modifier.align(Alignment.CenterHorizontally).width(32.dp).heightIn(min = 4.dp).clip(RoundedCornerShape(2.dp)).background(Mts.Line))
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(if (mode == CaptureMode.Task && subject == null && text == null) "New task" else "Capture", style = HqType.H2, color = Mts.Ink, modifier = Modifier.weight(1f))
                Box(Modifier.size(48.dp).clickable(role = Role.Button, onClickLabel = "Close", onClick = onClose), contentAlignment = Alignment.Center) { Icon(HqIcons.Close, null, Modifier.size(24.dp), tint = Mts.Ink) }
            }
            ModeControl(mode) { mode = it }
            Column(Modifier.weight(1f, fill = false).verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                when (mode) {
                    CaptureMode.Note -> {
                        preview?.let { PreviewCard(it) }
                        Field("Title", title, { title = it }, singleLine = true)
                        Field("Note", body, { body = it }, minHeight = 72.dp)
                        FolderField(folder, pickFolder, folders, { pickFolder = !pickFolder }) { folder = it; pickFolder = false }
                    }
                    CaptureMode.Task -> {
                        Field("Title", title, { title = it }, singleLine = true)
                        Field("Notes (optional)", body, { body = it }, minHeight = 72.dp)
                        Static("Where", "Personal · Inbox")
                        Row(Modifier.fillMaxWidth().heightIn(min = 48.dp).clickable(role = Role.Button) { more = !more }, verticalAlignment = Alignment.CenterVertically) {
                            Text("More options", style = HqType.BodySmBold, color = Mts.BrandBlue, modifier = Modifier.weight(1f))
                            Icon(if (more) HqIcons.ChevronUp else HqIcons.ChevronDown, null, Modifier.size(20.dp), tint = Mts.BrandBlue)
                        }
                        if (more) {
                            Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                                Priorities.forEach { (key, label) -> Pill(label, priority == key) { priority = key } }
                            }
                            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                                dueChoices().forEach { (value, label) -> Pill(label, due == value) { due = value } }
                            }
                            Field("Assign to", assignTo, { assignTo = it }, singleLine = true, hint = "Nobody. Naming a helper notifies it.")
                        }
                    }
                    CaptureMode.Chat -> {
                        preview?.let { PreviewCard(it) }
                        Field("Add to the open chat", body.ifBlank { title }, { body = it }, minHeight = 96.dp)
                    }
                }
            }
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Box(
                    Modifier.weight(1f).heightIn(min = 48.dp).clip(CircleShape).background(if (canSave) Mts.BrandGreen else Mts.Surface200).clickable(enabled = canSave, role = Role.Button) {
                        onSave(CaptureDraft(mode, title, body, folder, priority, due, assignTo), false)
                    },
                    contentAlignment = Alignment.Center,
                ) { Text(if (mode == CaptureMode.Chat) "Add to chat" else "Save", style = HqType.BodySmBold, color = if (canSave) Mts.OnGreen else Mts.InkMuted) }
                if (mode == CaptureMode.Note) Box(
                    Modifier.heightIn(min = 48.dp).clip(CircleShape).border(1.dp, Mts.Line, CircleShape).clickable(enabled = canSave, role = Role.Button) {
                        onSave(CaptureDraft(mode, title, body, folder), true)
                    }.padding(horizontal = 24.dp),
                    contentAlignment = Alignment.Center,
                ) { Text("Save and open", style = HqType.BodySmBold, color = Mts.Ink) }
            }
        }
    }
}

private fun dueChoices(): List<Pair<String?, String>> {
    val today = LocalDate.now()
    return listOf(null to "None", today.toString() to "Today", today.plusDays(1).toString() to "Tomorrow", today.plusDays(7).toString() to "Next week")
}

@Composable
private fun ModeControl(mode: CaptureMode, onMode: (CaptureMode) -> Unit) {
    val shape = CircleShape
    Row(Modifier.fillMaxWidth().heightIn(min = 48.dp).clip(shape).border(1.dp, Mts.Line, shape)) {
        CaptureMode.entries.forEachIndexed { i, m ->
            val on = m == mode
            Row(
                Modifier.weight(1f).heightIn(min = 48.dp).then(if (on) Modifier.background(Mts.Surface200) else Modifier).semantics { selected = on }.clickable(role = Role.Tab) { onMode(m) },
                horizontalArrangement = Arrangement.spacedBy(6.dp, Alignment.CenterHorizontally), verticalAlignment = Alignment.CenterVertically,
            ) {
                if (on) Icon(HqIcons.Check, null, Modifier.size(18.dp), tint = Mts.Ink)
                Text(m.label, style = HqType.BodySmBold, color = if (on) Mts.Ink else Mts.InkMuted)
            }
        }
    }
}

@Composable
private fun PreviewCard(p: CaptureWords.Preview) {
    val shape = RoundedCornerShape(12.dp)
    Row(Modifier.fillMaxWidth().clip(shape).border(1.dp, Mts.Hairline, shape).padding(12.dp), horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
        Icon(HqIcons.Link, null, Modifier.size(20.dp), tint = Mts.BrandBlue)
        Column {
            Text(p.title, style = HqType.BodySmBold, color = Mts.Ink, maxLines = 1, overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis)
            Text(p.where, style = HqType.Caption, color = Mts.InkMuted, maxLines = 1, overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis)
        }
    }
}

@Composable
private fun Field(label: String, value: String, onChange: (String) -> Unit, singleLine: Boolean = false, minHeight: androidx.compose.ui.unit.Dp = 56.dp, hint: String? = null) {
    var focused by remember { mutableStateOf(false) }
    val shape = RoundedCornerShape(12.dp)
    Column(
        Modifier.fillMaxWidth().heightIn(min = minHeight).clip(shape).border(if (focused) 2.dp else 1.dp, if (focused) Mts.BrandBlue else Mts.Line, shape).padding(horizontal = 16.dp, vertical = 10.dp),
    ) {
        Text(label, style = HqType.Caption.copy(fontWeight = androidx.compose.ui.text.font.FontWeight.Bold), color = Mts.InkMuted)
        BasicTextField(
            value, onChange, Modifier.fillMaxWidth().onFocusChanged { focused = it.isFocused }, singleLine = singleLine, textStyle = HqType.Body.copy(color = Mts.Ink), cursorBrush = SolidColor(Mts.Ink),
            decorationBox = { inner -> Box { if (value.isEmpty() && hint != null) Text(hint, style = HqType.BodySm, color = Mts.InkMuted); inner() } },
        )
    }
}

@Composable
private fun Static(label: String, value: String) {
    val shape = RoundedCornerShape(12.dp)
    Row(Modifier.fillMaxWidth().heightIn(min = 56.dp).clip(shape).border(1.dp, Mts.Line, shape).padding(horizontal = 16.dp, vertical = 10.dp), verticalAlignment = Alignment.CenterVertically) {
        Column(Modifier.weight(1f)) {
            Text(label, style = HqType.Caption.copy(fontWeight = androidx.compose.ui.text.font.FontWeight.Bold), color = Mts.InkMuted)
            Text(value, style = HqType.Body, color = Mts.Ink)
        }
    }
}

@Composable
private fun FolderField(folder: String, open: Boolean, folders: List<String>, onToggle: () -> Unit, onPick: (String) -> Unit) {
    val shape = RoundedCornerShape(12.dp)
    Column(Modifier.fillMaxWidth().clip(shape).border(1.dp, Mts.Line, shape)) {
        Row(Modifier.fillMaxWidth().heightIn(min = 56.dp).clickable(role = Role.Button, onClick = onToggle).padding(horizontal = 16.dp, vertical = 10.dp), verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                Text("Save in", style = HqType.Caption.copy(fontWeight = androidx.compose.ui.text.font.FontWeight.Bold), color = Mts.InkMuted)
                Text(folder, style = HqType.Body, color = Mts.Ink, maxLines = 1, overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis)
            }
            Icon(HqIcons.ChevronDown, null, Modifier.size(20.dp), tint = Mts.InkMuted)
        }
        if (open) Column(Modifier.heightIn(max = 200.dp).verticalScroll(rememberScrollState())) {
            folders.forEach { f -> Text(f.removePrefix("Notebooks/"), style = HqType.BodySm, color = if (f == folder) Mts.BrandGreen else Mts.Ink, modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp).clickable(role = Role.Button) { onPick(f) }.padding(horizontal = 16.dp, vertical = 12.dp)) }
        }
    }
}

@Composable
private fun Pill(label: String, selected: Boolean, onClick: () -> Unit) {
    Box(
        Modifier.heightIn(min = 36.dp).clip(CircleShape).then(if (selected) Modifier.background(Mts.Surface200).border(1.dp, Mts.BrandGreen, CircleShape) else Modifier.border(1.dp, Mts.Line, CircleShape)).clickable(role = Role.RadioButton, onClick = onClick).padding(horizontal = 12.dp),
        contentAlignment = Alignment.Center,
    ) { Text(label, style = HqType.BodySmBold, color = Mts.Ink, maxLines = 1) }
}

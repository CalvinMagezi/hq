package example.hq.spike.ui.chat

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import example.hq.spike.ui.chat.blocks.BlockActions
import example.hq.spike.ui.chat.blocks.Blocks
import example.hq.spike.ui.chat.blocks.HeaderLabel
import example.hq.spike.ui.chat.blocks.IconAction
import example.hq.spike.ui.chat.blocks.NoteText
import example.hq.spike.ui.chat.blocks.parseBlocks
import example.hq.spike.ui.shell.HqIcons
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts

private val SheetShape = RoundedCornerShape(topStart = 28.dp, topEnd = 28.dp)
private val SheetTop = 96.dp
private val Scrim = Color.Black.copy(alpha = 0.62f)
private val FadeHeight = 40.dp

/**
 * A note read in the chat, over a dimmed screen with its top edge 96 dp down. It is always the tall state of the
 * board; the half-height start the board mentions is not built (see TECHDEBT). Footer: Open in Notes, Ask about this.
 */
@Composable
fun NoteSheetView(sheet: NoteSheet, blocks: BlockActions, onClose: () -> Unit, onOpenInNotes: () -> Unit, onAsk: () -> Unit, onMenu: () -> Unit) {
    BackHandler(onBack = onClose)
    Box(Modifier.fillMaxSize()) {
        Box(Modifier.fillMaxSize().background(Scrim).clickable(interactionSource = remember { MutableInteractionSource() }, indication = null, onClick = onClose))
        Column(Modifier.align(Alignment.BottomCenter).fillMaxWidth().padding(top = SheetTop).fillMaxHeight().clip(SheetShape).background(Mts.Surface100)) {
            Box(Modifier.padding(top = 12.dp, bottom = 12.dp).size(width = 32.dp, height = 4.dp).align(Alignment.CenterHorizontally).background(Mts.Line, RoundedCornerShape(2.dp)))
            Header(sheet, onMenu)
            sheet.savedAge?.let { SavedPill(it) }
            NoteBody(sheet, blocks, Modifier.weight(1f))
            Footer(onOpenInNotes, onAsk)
        }
    }
}

@Composable
private fun Header(sheet: NoteSheet, onMenu: () -> Unit) {
    Row(Modifier.fillMaxWidth().padding(start = 16.dp, end = 4.dp, bottom = 8.dp), horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
        Box(Modifier.size(40.dp).background(Mts.Surface200, RoundedCornerShape(10.dp)), contentAlignment = Alignment.Center) {
            Icon(HqIcons.Notes, null, Modifier.size(22.dp), tint = Mts.Ink)
        }
        Column(Modifier.weight(1f)) {
            Text(sheet.title, style = HqType.ScreenTitle, color = Mts.Ink, maxLines = 1, overflow = TextOverflow.Ellipsis)
            if (sheet.trail.isNotBlank()) Text(sheet.trail, style = HqType.Caption.copy(fontSize = 13.sp, lineHeight = 18.sp), color = Mts.InkMuted, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
        IconAction(HqIcons.MoreVertical, "More", iconSize = 24.dp, onClick = onMenu)
    }
}

@Composable
private fun SavedPill(age: String) {
    Box(Modifier.padding(start = 16.dp, end = 16.dp, bottom = 12.dp)) {
        Text(
            (if (age.isBlank()) "Saved by HQ" else "Saved by HQ · $age"), style = HeaderLabel, color = Mts.InkMuted,
            modifier = Modifier.heightIn(min = 24.dp).border(1.dp, Mts.Line, CircleShape).padding(horizontal = 10.dp, vertical = 4.dp),
        )
    }
}

@Composable
private fun NoteBody(sheet: NoteSheet, blocks: BlockActions, modifier: Modifier) {
    val parsed = remember(sheet.body) { parseBlocks(NoteText.withoutFrontmatter(sheet.body)) }
    val scroll = rememberScrollState()
    Box(modifier.fillMaxWidth()) {
        Column(Modifier.fillMaxSize().verticalScroll(scroll).padding(start = 16.dp, top = 12.dp, end = 16.dp, bottom = FadeHeight)) {
            Blocks(parsed, blocks)
            if (sheet.truncated) {
                Text("This note is long. Open it in Notes to read all of it.", style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(top = 12.dp))
            }
        }
        Box(Modifier.fillMaxWidth().height(1.dp).background(Mts.Hairline))
        if (scroll.canScrollForward) {
            Box(Modifier.align(Alignment.BottomCenter).fillMaxWidth().height(FadeHeight).background(Brush.verticalGradient(listOf(Color.Transparent, Mts.Surface100))))
        }
    }
}

@Composable
private fun Footer(onOpenInNotes: () -> Unit, onAsk: () -> Unit) {
    Column(Modifier.fillMaxWidth()) {
        Box(Modifier.fillMaxWidth().height(1.dp).background(Mts.Hairline))
        Row(Modifier.fillMaxWidth().padding(start = 16.dp, top = 12.dp, end = 16.dp, bottom = 28.dp).navigationBarsPadding(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Box(
                Modifier.weight(1f).heightIn(min = 48.dp).clip(CircleShape).background(Mts.BrandGreen).clickable(role = Role.Button, onClick = onOpenInNotes).padding(horizontal = 24.dp),
                contentAlignment = Alignment.Center,
            ) { Text("Open in Notes", style = HqType.BodySmBold, color = Mts.OnGreen, maxLines = 1) }
            Box(
                Modifier.heightIn(min = 48.dp).clip(CircleShape).border(1.dp, Mts.Line, CircleShape).clickable(role = Role.Button, onClick = onAsk).padding(horizontal = 24.dp),
                contentAlignment = Alignment.Center,
            ) { Text("Ask about this", style = HqType.BodySmBold, color = Mts.Ink, maxLines = 1) }
        }
    }
}

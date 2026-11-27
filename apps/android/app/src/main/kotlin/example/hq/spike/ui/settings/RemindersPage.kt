package example.hq.spike.ui.settings

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
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
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import example.hq.spike.AlertPoster
import example.hq.spike.R
import example.hq.spike.data.HqServices
import example.hq.spike.data.HqUnavailable
import example.hq.spike.data.Reminder
import example.hq.spike.data.ReminderList
import example.hq.spike.data.createReminder
import example.hq.spike.data.doneReminder
import example.hq.spike.data.reminders
import example.hq.spike.data.snoozeReminder
import example.hq.spike.data.stopReminder
import example.hq.spike.ui.helpers.OutlineButton
import example.hq.spike.ui.inbox.PrimaryButton
import example.hq.spike.ui.reminders.RemindersLogic
import example.hq.spike.ui.reminders.RemindersLogic.Choice
import example.hq.spike.ui.reminders.RemindersLogic.Group
import example.hq.spike.ui.reminders.RemindersLogic.RepeatChoice
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts
import java.time.ZoneId
import java.time.ZonedDateTime
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

private const val POLL_MS = 20_000L

private sealed interface RemindersRead {
    data object Loading : RemindersRead
    data object Offline : RemindersRead
    data class Got(val list: ReminderList) : RemindersRead
}

@androidx.annotation.StringRes
private fun groupLabel(g: Group): Int = when (g) {
    Group.Overdue -> R.string.reminders_group_overdue
    Group.Today -> R.string.reminders_group_today
    Group.Tomorrow -> R.string.reminders_group_tomorrow
    Group.Later -> R.string.reminders_group_later
}

@androidx.annotation.StringRes
private fun choiceLabel(c: Choice): Int = when (c) {
    Choice.Hour -> R.string.reminders_snooze_hour
    Choice.Evening -> R.string.reminders_snooze_evening
    Choice.Tomorrow -> R.string.reminders_snooze_tomorrow
    Choice.Monday -> R.string.reminders_snooze_monday
}

@androidx.annotation.StringRes
private fun repeatLabel(r: RepeatChoice): Int = when (r) {
    RepeatChoice.None -> R.string.reminders_repeat_none
    RepeatChoice.Daily -> R.string.reminders_repeat_daily
    RepeatChoice.Weekdays -> R.string.reminders_repeat_weekdays
    RepeatChoice.Weekly -> R.string.reminders_repeat_weekly
    RepeatChoice.Monthly -> R.string.reminders_repeat_monthly
}

/**
 * Reminders: what is coming (overdue, today, tomorrow, later) and what finished, a form for a new one, and whether a reminder's
 * words show on the lock screen. RemindersRead from HQ every 20 seconds while the page is open. Times are the owner's own zone: the one HQ
 * saved, else this phone's.
 */
@Composable
internal fun RemindersPage() {
    val context = LocalContext.current
    val client = remember { HqServices.get(context).client }
    val scope = rememberCoroutineScope()
    var finished by rememberSaveable { mutableStateOf(false) }
    var adding by rememberSaveable { mutableStateOf(false) }
    var read by remember { mutableStateOf<RemindersRead>(RemindersRead.Loading) }
    var failure by remember { mutableStateOf<String?>(null) }
    var showText by remember { mutableStateOf(AlertPoster.showReminderText(context)) }
    var reload by remember { mutableStateOf(0) }
    LaunchedEffect(finished, reload) {
        while (true) {
            read = try { RemindersRead.Got(client.reminders(done = finished)) } catch (e: HqUnavailable) { RemindersRead.Offline }
            delay(POLL_MS)
        }
    }
    val zone = RemindersLogic.zoneOf((read as? RemindersRead.Got)?.list?.timeZone)
    fun change(job: suspend () -> Unit) {
        scope.launch {
            failure = null
            try { job(); reload++ } catch (e: HqUnavailable) { failure = e.serverMessage ?: context.getString(R.string.reminders_hq_offline) }
        }
    }
    Page {
        Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            ChoiceChip(stringResource(R.string.reminders_tab_upcoming), !finished) { finished = false }
            ChoiceChip(stringResource(R.string.reminders_tab_done), finished) { finished = true }
        }
        if (!finished) {
            Box(Modifier.padding(horizontal = 16.dp, vertical = 4.dp)) {
                PrimaryButton(stringResource(R.string.reminders_new), Modifier.fillMaxWidth()) { adding = !adding }
            }
        }
        if (adding && !finished) {
            NewReminderForm(zone, onSet = { title, at, repeat, nudge ->
                change {
                    client.createReminder(title, RemindersLogic.localString(at), zone.id, repeat, nudge)
                    adding = false
                }
            })
        }
        failure?.let { Text(it, style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp)) }
        when (val r = read) {
            RemindersRead.Loading -> Text(stringResource(R.string.reminders_loading), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(16.dp))
            RemindersRead.Offline -> Text(stringResource(R.string.reminders_hq_offline), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(16.dp))
            is RemindersRead.Got -> {
                val saved = r.list.timeZone
                Text(
                    if (saved != null) stringResource(R.string.reminders_in_zone, saved) else stringResource(R.string.reminders_no_zone),
                    style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp),
                )
                if (r.list.reminders.isEmpty()) {
                    Text(stringResource(if (finished) R.string.reminders_empty_done else R.string.reminders_empty), style = HqType.Body, color = Mts.InkMuted, modifier = Modifier.padding(16.dp))
                } else if (finished) {
                    Group { r.list.reminders.forEachIndexed { i, rem -> if (i > 0) Divider(); FinishedRow(rem) } }
                } else {
                    val now = remember(r) { ZonedDateTime.now(zone) }
                    RemindersLogic.grouped(r.list.reminders, now).forEach { (group, rows) ->
                        Label(stringResource(groupLabel(group)))
                        Group {
                            rows.forEachIndexed { i, rem ->
                                if (i > 0) Divider()
                                ReminderRow(
                                    rem, now,
                                    onDone = { change { client.doneReminder(rem.id) } },
                                    onSnooze = { until -> change { client.snoozeReminder(rem.id, until) } },
                                    onStop = { change { client.stopReminder(rem.id) } },
                                )
                            }
                        }
                    }
                }
            }
        }
        Label(stringResource(R.string.row_notifications))
        Group {
            Row(
                Modifier.fillMaxWidth().heightIn(min = 64.dp).padding(horizontal = 16.dp, vertical = 8.dp),
                horizontalArrangement = Arrangement.spacedBy(16.dp), verticalAlignment = Alignment.CenterVertically,
            ) {
                Column(Modifier.weight(1f)) {
                    Text(stringResource(R.string.settings_reminder_text_title), style = HqType.BodyBold, color = Mts.Ink)
                    Text(stringResource(R.string.settings_reminder_text_note), style = HqType.BodySm, color = Mts.InkMuted)
                }
                Switch(
                    checked = showText,
                    onCheckedChange = { showText = it; AlertPoster.setShowReminderText(context, it) },
                    colors = SwitchDefaults.colors(
                        checkedTrackColor = Mts.BrandGreen, checkedThumbColor = Mts.OnGreen, checkedBorderColor = Mts.BrandGreen,
                        uncheckedTrackColor = Mts.Surface200, uncheckedThumbColor = Mts.Line, uncheckedBorderColor = Mts.Line,
                    ),
                )
            }
        }
    }
}

@Composable
private fun ReminderRow(r: Reminder, now: ZonedDateTime, onDone: () -> Unit, onSnooze: (Long) -> Unit, onStop: () -> Unit) {
    val hour = now.plusMinutes(RemindersLogic.SNOOZE_HOUR_MINUTES).toEpochSecond()
    val tomorrow = RemindersLogic.tomorrowMorning(now).toEpochSecond()
    Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 12.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Text(r.title, style = HqType.BodyBold, color = Mts.Ink)
        Text(r.due, style = HqType.BodySm, color = Mts.InkMuted)
        val tags = listOfNotNull(r.repeat, if (r.nudge) stringResource(R.string.reminders_keeps_asking) else null, if (r.tainted) stringResource(R.string.reminders_set_outside) else null)
        if (tags.isNotEmpty()) Text(tags.joinToString(" · "), style = HqType.BodySm, color = Mts.InkMuted)
        r.note?.let { Text(it, style = HqType.BodySm, color = Mts.Ink) }
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
            PrimaryButton(stringResource(R.string.reminders_mark_done), Modifier.weight(1f), onClick = onDone)
            OutlineButton(stringResource(R.string.reminders_snooze_hour), Modifier.weight(1f), maxLines = 2) { onSnooze(hour) }
            OutlineButton(stringResource(R.string.reminders_snooze_tomorrow), Modifier.weight(1f), maxLines = 2) { onSnooze(tomorrow) }
        }
        OutlineButton(stringResource(R.string.reminders_stop), Modifier.fillMaxWidth(), onClick = onStop)
    }
}

@Composable
private fun FinishedRow(r: Reminder) {
    val status = when (r.status) {
        "cancelled" -> R.string.reminders_status_cancelled
        "lapsed" -> R.string.reminders_status_lapsed
        else -> R.string.reminders_status_done
    }
    Column(Modifier.fillMaxWidth().heightIn(min = 56.dp).padding(horizontal = 16.dp, vertical = 8.dp), verticalArrangement = Arrangement.Center) {
        Text(r.title, style = HqType.BodyBold, color = Mts.Ink)
        Text(stringResource(status), style = HqType.BodySm, color = Mts.InkMuted)
    }
}

@Composable
private fun ChoiceChip(label: String, selected: Boolean, onClick: () -> Unit) {
    Box(
        Modifier.heightIn(min = 40.dp).clip(CircleShape)
            .background(if (selected) Mts.BrandGreen else Mts.Surface000)
            .border(1.dp, if (selected) Mts.BrandGreen else Mts.Line, CircleShape)
            .semantics { this.selected = selected }
            .clickable(role = Role.RadioButton, onClick = onClick).padding(horizontal = 16.dp, vertical = 8.dp),
        contentAlignment = Alignment.Center,
    ) {
        Text(label, style = HqType.BodyBold, color = if (selected) Mts.OnGreen else Mts.Ink)
    }
}

/** A new reminder: what, when (a quick time or a picked day and time), whether it repeats, and whether to keep asking. */
@Composable
private fun NewReminderForm(zone: ZoneId, onSet: (title: String, at: ZonedDateTime, repeat: String?, nudge: Boolean) -> Unit) {
    val context = LocalContext.current
    val now = remember { ZonedDateTime.now(zone) }
    val options = remember { RemindersLogic.options(now, monday = true) }
    var title by rememberSaveable { mutableStateOf("") }
    var chosen by rememberSaveable { mutableStateOf(Choice.Hour.name) }
    var picked by remember { mutableStateOf<ZonedDateTime?>(null) }
    var repeat by rememberSaveable { mutableStateOf(RepeatChoice.None.name) }
    var nudge by rememberSaveable { mutableStateOf(false) }
    val at = if (chosen == PICK) picked else options.firstOrNull { it.choice.name == chosen }?.at
    fun pick() {
        val base = picked ?: now
        android.app.DatePickerDialog(context, { _, y, m, d ->
            android.app.TimePickerDialog(context, { _, h, min -> picked = ZonedDateTime.of(y, m + 1, d, h, min, 0, 0, zone); chosen = PICK }, base.hour, base.minute, android.text.format.DateFormat.is24HourFormat(context)).show()
        }, base.year, base.monthValue - 1, base.dayOfMonth).show()
    }
    Column(Modifier.fillMaxWidth().padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Text(stringResource(R.string.reminders_what_label), style = HqType.BodyBold, color = Mts.Ink)
        Box(Modifier.fillMaxWidth().clip(RoundedCornerShape(12.dp)).border(1.dp, Mts.Line, RoundedCornerShape(12.dp)).padding(12.dp)) {
            BasicTextField(title, { title = it.take(MAX_TITLE) }, Modifier.fillMaxWidth(), textStyle = HqType.Body.copy(color = Mts.Ink), cursorBrush = SolidColor(Mts.Ink))
        }
        Text(stringResource(R.string.reminders_when_label), style = HqType.BodyBold, color = Mts.Ink)
        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            options.chunked(2).forEach { row ->
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) { row.forEach { o -> ChoiceChip(stringResource(choiceLabel(o.choice)), chosen == o.choice.name) { chosen = o.choice.name } } }
            }
            ChoiceChip(stringResource(R.string.reminders_when_pick), chosen == PICK) { pick() }
        }
        at?.let { Text(stringResource(R.string.reminders_picked, DateTimeFormatter.ofLocalizedDateTime(FormatStyle.MEDIUM, FormatStyle.SHORT).format(it)), style = HqType.BodySm, color = Mts.InkMuted) }
        Text(stringResource(R.string.reminders_repeat_label), style = HqType.BodyBold, color = Mts.Ink)
        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            RepeatChoice.entries.chunked(2).forEach { row ->
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) { row.forEach { r -> ChoiceChip(stringResource(repeatLabel(r)), repeat == r.name) { repeat = r.name } } }
            }
        }
        Row(horizontalArrangement = Arrangement.spacedBy(16.dp), verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                Text(stringResource(R.string.reminders_nudge_label), style = HqType.BodyBold, color = Mts.Ink)
                Text(stringResource(R.string.reminders_nudge_note), style = HqType.BodySm, color = Mts.InkMuted)
            }
            Switch(
                checked = nudge, onCheckedChange = { nudge = it },
                colors = SwitchDefaults.colors(
                    checkedTrackColor = Mts.BrandGreen, checkedThumbColor = Mts.OnGreen, checkedBorderColor = Mts.BrandGreen,
                    uncheckedTrackColor = Mts.Surface200, uncheckedThumbColor = Mts.Line, uncheckedBorderColor = Mts.Line,
                ),
            )
        }
        PrimaryButton(stringResource(R.string.reminders_set), Modifier.fillMaxWidth(), enabled = title.isNotBlank() && at != null) {
            val first = at ?: return@PrimaryButton
            onSet(title.trim(), first, RemindersLogic.repeatSpec(RepeatChoice.valueOf(repeat), first), nudge)
        }
    }
}

private const val PICK = "Pick"
private const val MAX_TITLE = 200

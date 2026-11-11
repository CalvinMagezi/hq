package example.hq.spike.ui.settings

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
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
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import example.hq.spike.R
import example.hq.spike.calendar.AndroidCalendarStore
import example.hq.spike.calendar.CalendarPrefs
import example.hq.spike.calendar.WritableCalendar
import example.hq.spike.ui.shell.HqIcons
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

private val CALENDAR_PERMISSIONS = arrayOf(android.Manifest.permission.READ_CALENDAR, android.Manifest.permission.WRITE_CALENDAR)

/** "Calendar HQ saves events to": asks for access when it is missing, then lists the calendars that can take new events. */
@Composable
internal fun CalendarPage() {
    val c = LocalContext.current.applicationContext
    val prefs = remember { CalendarPrefs(c) }
    val store = remember { AndroidCalendarStore(c) }
    var allowed by remember { mutableStateOf(AndroidCalendarStore.hasPermission(c)) }
    var asked by remember { mutableStateOf(false) }
    var calendars by remember { mutableStateOf<List<WritableCalendar>>(emptyList()) }
    var chosen by remember { mutableStateOf(prefs.get()) }
    val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) {
        allowed = AndroidCalendarStore.hasPermission(c)
        asked = true
    }
    LaunchedEffect(allowed) {
        calendars = if (allowed) withContext(Dispatchers.IO) { store.writableCalendars() } else emptyList()
    }
    Page {
        Text(stringResource(R.string.cal_intro), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(start = 20.dp, top = 8.dp, end = 20.dp))
        if (!allowed) {
            Notice(stringResource(if (asked) R.string.cal_denied_note else R.string.cal_need_access), HqIcons.Info, warn = asked)
            Column(Modifier.padding(horizontal = 16.dp, vertical = 12.dp)) { CardButton(stringResource(R.string.cal_allow), true) { ask.launch(CALENDAR_PERMISSIONS) } }
            return@Page
        }
        Label(stringResource(R.string.cal_group_calendars))
        if (calendars.isEmpty()) {
            Notice(stringResource(R.string.cal_none_writable), HqIcons.Info)
            return@Page
        }
        Group {
            calendars.forEachIndexed { i, cal ->
                if (i > 0) Divider()
                CalendarChoice(cal, picked = cal.id == chosen) { prefs.setPreferred(cal.id); chosen = cal.id }
            }
        }
    }
}

@Composable
private fun CalendarChoice(cal: WritableCalendar, picked: Boolean, onPick: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().heightIn(min = 56.dp).clickable(role = Role.RadioButton, onClick = onPick).padding(horizontal = 16.dp, vertical = 8.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f)) {
            Text(cal.displayName, style = HqType.BodyBold, color = Mts.Ink)
            if (cal.accountLabel.isNotBlank()) Text(cal.accountLabel, style = HqType.BodySm, color = Mts.InkMuted)
        }
        if (picked) Icon(HqIcons.Check, stringResource(R.string.cal_chosen), Modifier.size(20.dp), tint = Mts.BrandGreen)
    }
}

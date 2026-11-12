package example.hq.spike.ui.settings

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import example.hq.spike.R
import example.hq.spike.contacts.AndroidContactsReader
import example.hq.spike.ui.shell.HqIcons
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts

/** "Contacts HQ can look up": asks for read access when it is missing, and says so once it is allowed. */
@Composable
internal fun ContactsPage() {
    val c = LocalContext.current.applicationContext
    var allowed by remember { mutableStateOf(AndroidContactsReader.hasPermission(c)) }
    var asked by remember { mutableStateOf(false) }
    val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) {
        allowed = AndroidContactsReader.hasPermission(c)
        asked = true
    }
    Page {
        Text(stringResource(R.string.contacts_intro), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(start = 20.dp, top = 8.dp, end = 20.dp))
        if (allowed) {
            Notice(stringResource(R.string.contacts_allowed_note), HqIcons.Check)
            return@Page
        }
        Notice(stringResource(if (asked) R.string.contacts_denied_note else R.string.contacts_need_access), HqIcons.Info, warn = asked)
        Column(Modifier.padding(horizontal = 16.dp, vertical = 12.dp)) { CardButton(stringResource(R.string.contacts_allow), true) { ask.launch(android.Manifest.permission.READ_CONTACTS) } }
    }
}

package example.hq.spike

import android.app.Activity
import android.content.Intent
import android.os.Bundle
import example.hq.spike.ui.shell.Links

/**
 * Takes an hq:// link from outside the app and hands it to the shell. It passes on the link text and nothing else. The shell is
 * the launcher, so other apps can start it too, but from them it takes only the same link, read as a place to show, never as
 * something to do: its design-check extras work only in a debuggable build, and a chat draft comes only from the capture sheet.
 */
class LinkActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val link = intent?.dataString
        startActivity(Intent(this, ShellActivity::class.java).putExtra(Links.EXTRA, link).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP))
        finish()
        overridePendingTransition(0, 0)
    }
}

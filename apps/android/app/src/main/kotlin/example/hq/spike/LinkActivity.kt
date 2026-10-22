package example.hq.spike

import android.app.Activity
import android.content.Intent
import android.os.Bundle
import example.hq.spike.ui.shell.Links

/**
 * Takes an hq:// link from outside the app and hands it to the shell. The shell is closed to other apps, so this is the only door:
 * it passes on the link text and nothing else, and the shell reads it as a place to show, never as something to do.
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

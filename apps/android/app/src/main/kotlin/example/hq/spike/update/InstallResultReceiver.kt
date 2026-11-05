package example.hq.spike.update

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageInstaller

/**
 * Receives Android's answer to an install session. When it needs the owner's confirmation, the screen starts only while the app is
 * in front: a window that opens over something else is easy to dismiss by mistake, so then the "downloaded" notice carries it.
 */
class InstallResultReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val status = intent.getIntExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE)
        if (status == PackageInstaller.STATUS_PENDING_USER_ACTION) {
            val confirm = if (android.os.Build.VERSION.SDK_INT >= 33) intent.getParcelableExtra(Intent.EXTRA_INTENT, Intent::class.java) else @Suppress("DEPRECATION") intent.getParcelableExtra(Intent.EXTRA_INTENT)
            if (confirm != null && AppVisibility.visible) context.startActivity(confirm.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            else Updater.confirmLater(context)
            return
        }
        Updater.finished(status, intent.getStringExtra(PackageInstaller.EXTRA_STATUS_MESSAGE))
    }
}

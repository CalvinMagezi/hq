package example.hq.spike.ui.pairing

import android.app.Activity
import android.content.Intent
import android.os.Bundle

/**
 * The door for `hqpair://` links: a QR code read by the stock camera, a browser or another app. It
 * checks the shape of the link and passes the text on; the pairing screen then asks the person
 * before anything is dialed, because anyone can send a link like this.
 */
class PairLinkActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val link = PairLink.accept(intent?.dataString)
        startActivity(
            Intent(this, PairingActivity::class.java)
                .apply { if (link != null) putExtra(PairingActivity.EXTRA_LINK, link) }
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP),
        )
        finish()
        overridePendingTransition(0, 0)
    }
}

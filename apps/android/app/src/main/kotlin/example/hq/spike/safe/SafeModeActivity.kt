package example.hq.spike.safe

import android.app.Activity
import android.app.AlertDialog
import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.graphics.Typeface
import android.net.Uri
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.ResultReceiver
import android.provider.DocumentsContract
import android.provider.OpenableColumns
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.view.WindowManager
import android.widget.Button
import android.widget.LinearLayout
import android.widget.ProgressBar
import android.widget.ScrollView
import android.widget.TextView
import androidx.compose.ui.graphics.toArgb
import example.hq.spike.HubService
import example.hq.spike.R
import example.hq.spike.ServerHq
import example.hq.spike.ui.theme.Mts
import java.io.File
import java.time.LocalDateTime
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

/**
 * Saves a copy of the notes into one .zip in a place the owner picks. It runs in its own process (`:safe`) and uses Views only, so a
 * crash in HQ's own screens, or in Compose, cannot take it down. It reads no extras: whatever started it, it behaves the same.
 */
class SafeModeActivity : Activity() {
    private val main = Handler(Looper.getMainLooper())
    private lateinit var root: LinearLayout
    private var plan: NotesCopy.Plan? = null
    private var counting = 0
    private var serverOnly = false
    private var note: String? = null

    @Volatile private var cancel = false

    @Volatile private var copying = false
    private var progressBar: ProgressBar? = null
    private var filesLine: TextView? = null
    private var bytesLine: TextView? = null
    private var timeLine: TextView? = null
    private var holdLine: TextView? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        root = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL; setPadding(dp(20), dp(24), dp(20), dp(24)) }
        setContentView(ScrollView(this).apply {
            setBackgroundColor(Mts.Surface000.toArgb())
            isFillViewport = true
            addView(root, ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT)
        })
        val left = NotesCopy.pending(filesDir)
        if (left != null) {
            runCatching { DocumentsContract.deleteDocument(contentResolver, Uri.parse(left)) }
            NotesCopy.clearPending(filesDir)
            note = getString(R.string.safe_interrupted)
        }
        val hasVault = File(filesDir, NotesCopy.VAULT).isDirectory
        serverOnly = !hasVault && ServerHq.current(this) != null
        intro()
        Thread {
            val p = NotesCopy.plan(filesDir, withVault = !serverOnly) { n -> main.post { if (!copying && plan == null) { counting = n; updateCount() } } }
            main.post { plan = p; if (!copying && !isFinishing) intro() }
        }.start()
    }

    private var countLine: TextView? = null
    private fun updateCount() { countLine?.text = getString(R.string.safe_counting, counting.toString()) }

    override fun onBackPressed() {
        if (copying) stopDialog() else super.onBackPressed()
    }

    private fun intro() {
        reset()
        val crashed = CrashMarker.tripped(filesDir)
        if (crashed) body(getString(R.string.safe_after_crashes), Mts.Amber.toArgb())
        note?.let { body(it, Mts.Amber.toArgb()) }
        val p = plan
        if (serverOnly) {
            title(R.string.safe_server_title)
            body(getString(R.string.safe_server_body))
            val n = p?.waiting ?: 0
            if (p == null) { countLine = body(getString(R.string.safe_counting, counting.toString())); return }
            if (n > 0) {
                group(R.string.safe_group_phone)
                body(resources.getQuantityString(R.plurals.safe_waiting_count, n, n.toString()))
                body(getString(R.string.safe_waiting_body))
                primary(resources.getQuantityString(R.plurals.safe_save_waiting, n, n.toString())) { choose() }
            }
            secondary(getString(R.string.safe_not_now)) { leave() }
            return
        }
        title(R.string.safe_title)
        body(getString(R.string.safe_body))
        group(R.string.safe_group_in)
        bullet(R.string.safe_in_notes)
        bullet(R.string.safe_in_database)
        if ((p?.waiting ?: NotesCopy.waitingCount(filesDir)) > 0) bullet(R.string.safe_in_waiting)
        group(R.string.safe_group_out)
        bullet(R.string.safe_out_keys)
        bullet(R.string.safe_out_settings)
        if (p == null) {
            countLine = body(getString(R.string.safe_counting, counting.toString()))
        } else {
            body(resources.getQuantityString(R.plurals.safe_size, p.files, NotesCopy.size(p.bytes), p.files.toString()))
        }
        body(getString(R.string.safe_lock_line))
        if (p != null) primary(getString(R.string.safe_choose_place)) { choose() }
        if (crashed) secondary(getString(R.string.safe_open_hq_again)) {
            CrashMarker.reset(filesDir)
            startActivity(Intent().setClassName(packageName, "example.hq.spike.ShellActivity").addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            finish()
        }
        secondary(getString(R.string.safe_not_now)) { leave() }
    }

    private fun leave() {
        CrashMarker.reset(filesDir)
        finish()
    }

    private fun choose() {
        note = null
        val name = NotesCopy.fileName(LocalDateTime.now())
        val i = Intent(Intent.ACTION_CREATE_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE).setType("application/zip").putExtra(Intent.EXTRA_TITLE, name)
        try { startActivityForResult(i, PICK) } catch (_: ActivityNotFoundException) { fail(getString(R.string.safe_fail_place_gone), false) }
    }

    @Deprecated("Deprecated in Java")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode != PICK) return
        val uri = data?.data
        if (resultCode != RESULT_OK || uri == null) { intro(); return }
        start(uri)
    }

    private fun placeTitle(uri: Uri): String = roots(uri)?.first ?: getString(R.string.safe_place_unknown)

    /** The title of the place the document is on, and the bytes it reports free; null when the provider says nothing. */
    private fun roots(uri: Uri): Pair<String, Long?>? = try {
        val docId = DocumentsContract.getDocumentId(uri)
        var best: Pair<String, Long?>? = null
        var bestLen = -1
        contentResolver.query(DocumentsContract.buildRootsUri(uri.authority), null, null, null, null)?.use { c ->
            val doc = c.getColumnIndex(DocumentsContract.Root.COLUMN_DOCUMENT_ID)
            val title = c.getColumnIndex(DocumentsContract.Root.COLUMN_TITLE)
            val avail = c.getColumnIndex(DocumentsContract.Root.COLUMN_AVAILABLE_BYTES)
            while (c.moveToNext()) {
                val rootDoc = if (doc >= 0) c.getString(doc).orEmpty() else ""
                val matches = rootDoc.isNotEmpty() && docId.startsWith(rootDoc.substringBefore(':'))
                if (matches && rootDoc.length > bestLen) {
                    bestLen = rootDoc.length
                    best = (if (title >= 0) c.getString(title).orEmpty() else "") to (if (avail >= 0 && !c.isNull(avail)) c.getLong(avail) else null)
                }
            }
        }
        best
    } catch (_: Exception) {
        null
    }

    private fun start(uri: Uri) {
        val p = plan ?: return
        val info = roots(uri)
        val free = info?.second
        val place = info?.first?.ifBlank { null } ?: getString(R.string.safe_place_unknown)
        if (NotesCopy.short(p, free)) {
            runCatching { DocumentsContract.deleteDocument(contentResolver, uri) }
            val local = uri.authority == "com.android.externalstorage.documents" && DocumentsContract.getDocumentId(uri).startsWith("primary")
            val msg = if (local) getString(R.string.safe_fail_phone_full, NotesCopy.size(NotesCopy.freeUpBytes(p, free!!)))
            else getString(R.string.safe_fail_needs, NotesCopy.size(p.bytes + NotesCopy.SPACE_MARGIN), place, NotesCopy.size(free!!))
            fail(msg, false)
            return
        }
        NotesCopy.markPending(filesDir, uri.toString())
        cancel = false
        copying = true
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        saving(p)
        Thread { run(p, uri) }.start()
    }

    private fun run(p: NotesCopy.Plan, uri: Uri) {
        var held = false
        try {
            if (p.databaseFiles > 0) {
                val answer = hold()
                held = answer == HubService.COPY_HELD
                main.post { holdLine?.text = getString(if (held) R.string.safe_hq_paused else R.string.safe_no_pause); holdLine?.visibility = if (answer == HubService.COPY_NOT_RUNNING) View.GONE else View.VISIBLE }
            }
            val out = contentResolver.openOutputStream(uri, "w") ?: throw java.io.IOException("No output")
            val result = out.buffered(1 shl 16).use { stream ->
                NotesCopy.write(p, stream, cancelled = { cancel }, onDatabaseCopied = { if (held) { release(); held = false } }) { pr -> main.post { progress(pr) } }
            }
            NotesCopy.clearPending(filesDir)
            NotesCopy.recordCopy(filesDir, System.currentTimeMillis())
            val name = displayName(uri)
            main.post { copying = false; window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON); done(result, name, placeTitle(uri)) }
        } catch (e: NotesCopy.Cancelled) {
            cleanup(uri)
            main.post { copying = false; window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON); cancelled() }
        } catch (e: Exception) {
            cleanup(uri)
            val why = NotesCopy.classify(e)
            main.post {
                copying = false
                window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
                fail(getString(if (why == NotesCopy.Failure.PlaceFull) R.string.safe_fail_place_full else R.string.safe_fail_place_gone), true)
            }
        } finally {
            if (held) release()
        }
    }

    private fun cleanup(uri: Uri) {
        runCatching { DocumentsContract.deleteDocument(contentResolver, uri) }
        NotesCopy.clearPending(filesDir)
    }

    private fun hold(): Int {
        val latch = CountDownLatch(1)
        var code = HubService.COPY_FAILED
        val receiver = object : ResultReceiver(main) {
            override fun onReceiveResult(resultCode: Int, resultData: Bundle?) { code = resultCode; latch.countDown() }
        }
        return try {
            startService(Intent(this, HubService::class.java).setAction(HubService.ACTION_COPY_HOLD).putExtra(HubService.EXTRA_REPLY, receiver))
            if (latch.await(30, TimeUnit.SECONDS)) code else HubService.COPY_FAILED
        } catch (_: Exception) {
            HubService.COPY_FAILED
        }
    }

    private fun release() {
        runCatching { startService(Intent(this, HubService::class.java).setAction(HubService.ACTION_COPY_RELEASE)) }
    }

    private fun displayName(uri: Uri): String = try {
        contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { if (it.moveToFirst()) it.getString(0) else null }
    } catch (_: Exception) { null } ?: NotesCopy.fileName(LocalDateTime.now())

    private fun saving(p: NotesCopy.Plan) {
        reset()
        title(R.string.safe_saving_title)
        val bar = ProgressBar(this, null, android.R.attr.progressBarStyleHorizontal).apply { max = 1000; progress = 0 }
        root.addView(bar, LinearLayout.LayoutParams(-1, dp(12)).apply { topMargin = dp(16); bottomMargin = dp(8) })
        progressBar = bar
        filesLine = body(getString(R.string.safe_progress_files, "0", p.files.toString()))
        bytesLine = body("")
        timeLine = body("")
        body(getString(R.string.safe_keep_open))
        holdLine = body("").also { it.visibility = View.GONE }
        secondary(getString(R.string.safe_stop_copying)) { stopDialog() }
    }

    private fun progress(p: NotesCopy.Progress) {
        if (!copying) return
        progressBar?.progress = if (p.bytesTotal > 0) (p.bytesDone * 1000 / p.bytesTotal).toInt().coerceIn(0, 1000) else 0
        filesLine?.text = getString(R.string.safe_progress_files, p.filesDone.toString(), p.filesTotal.toString())
        val pct = if (p.bytesTotal > 0) (p.bytesDone * 100 / p.bytesTotal).coerceIn(0, 100) else 0
        bytesLine?.text = getString(R.string.safe_progress_bytes, "$pct%", (p.bytesDone / 1_000_000).toString(), (p.bytesTotal / 1_000_000).toString())
        timeLine?.text = NotesCopy.secondsLeft(p)?.let { getString(R.string.safe_time_left, duration(it)) }.orEmpty()
    }

    private fun duration(s: Long): String = if (s >= 90) "${(s + 30) / 60} min" else "$s s"

    private fun stopDialog() {
        AlertDialog.Builder(this).setTitle(R.string.safe_stop_title).setMessage(R.string.safe_stop_body)
            .setPositiveButton(R.string.safe_keep_copying, null)
            .setNegativeButton(R.string.safe_stop_copying) { _, _ -> cancel = true }
            .show()
    }

    private fun done(r: NotesCopy.Result, name: String, place: String) {
        reset()
        title(R.string.safe_done_title)
        group(R.string.safe_done_file)
        body(name)
        group(R.string.safe_done_size)
        body(resources.getQuantityString(R.plurals.safe_done_size_value, r.files, NotesCopy.size(r.zipBytes), r.files.toString()))
        group(R.string.safe_done_saved_to)
        val time = LocalDateTime.now().let { "%02d:%02d".format(it.hour, it.minute) }
        body(getString(R.string.safe_done_place, place, time))
        if (r.leftOut.isNotEmpty()) body(resources.getQuantityString(R.plurals.safe_left_out, r.leftOut.size, r.leftOut.size.toString()), Mts.Amber.toArgb())
        body(getString(R.string.safe_done_move))
        body(getString(R.string.safe_done_restore))
        primary(getString(R.string.safe_done_button)) { leave() }
    }

    private fun cancelled() {
        reset()
        title(R.string.safe_stop_title)
        body(getString(R.string.safe_cancelled))
        primary(getString(R.string.safe_choose_place)) { choose() }
        secondary(getString(R.string.safe_not_now)) { leave() }
    }

    private fun fail(message: String, removed: Boolean) {
        reset()
        title(R.string.safe_title)
        body(message, Mts.DangerText.toArgb())
        if (removed) body(getString(R.string.safe_fail_removed))
        primary(getString(R.string.safe_choose_another)) { choose() }
        secondary(getString(R.string.safe_not_now)) { leave() }
    }

    private fun reset() {
        root.removeAllViews()
        countLine = null
    }

    private fun title(res: Int) = text(getString(res), 22f, Mts.Ink.toArgb(), true, 8)
    private fun group(res: Int) = text(getString(res), 13f, Mts.InkMuted.toArgb(), true, 20)
    private fun bullet(res: Int) = text(getString(res), 16f, Mts.Ink.toArgb(), false, 6)
    private fun body(s: String, color: Int = Mts.Ink.toArgb()) = text(s, 16f, color, false, 12)

    private fun text(s: String, sp: Float, color: Int, bold: Boolean, top: Int): TextView {
        val t = TextView(this).apply { text = s; textSize = sp; setTextColor(color); if (bold) setTypeface(typeface, Typeface.BOLD) }
        root.addView(t, LinearLayout.LayoutParams(-1, -2).apply { topMargin = dp(top) })
        return t
    }

    private fun primary(label: String, click: () -> Unit) = button(label, Mts.BrandBlue.toArgb(), Mts.Surface000.toArgb(), click)
    private fun secondary(label: String, click: () -> Unit) = button(label, Mts.Surface200.toArgb(), Mts.Ink.toArgb(), click)

    private fun button(label: String, bg: Int, fg: Int, click: () -> Unit) {
        val b = Button(this).apply { text = label; isAllCaps = false; setBackgroundColor(bg); setTextColor(fg); minHeight = dp(48); gravity = Gravity.CENTER; setOnClickListener { click() } }
        root.addView(b, LinearLayout.LayoutParams(-1, -2).apply { topMargin = dp(16) })
    }

    private fun dp(v: Int) = (v * resources.displayMetrics.density).toInt()

    companion object {
        private const val PICK = 41

        /** Opens the screen from another screen of the app. Nothing is passed in: the screen works out the rest itself. */
        fun open(c: Context) {
            c.startActivity(Intent(c, SafeModeActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        }
    }
}

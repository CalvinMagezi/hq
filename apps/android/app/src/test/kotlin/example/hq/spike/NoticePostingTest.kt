package example.hq.spike

import android.app.KeyguardManager
import android.app.Notification
import android.app.NotificationManager
import android.content.Context
import androidx.test.core.app.ApplicationProvider
import example.hq.spike.ui.inbox.CardIcon
import example.hq.spike.ui.inbox.CardKind
import example.hq.spike.ui.inbox.InboxCard
import example.hq.spike.ui.inbox.InboxWords
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

/** What a notification says and which buttons it carries, for every case of rules 1 to 11 of wave-0/01-lock-screen.md. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class NoticePostingTest {
    private val c: Context = ApplicationProvider.getApplicationContext()
    private val nm get() = c.getSystemService(NotificationManager::class.java)

    @Before fun fresh() {
        c.getSharedPreferences("alerts", Context.MODE_PRIVATE).edit().clear().commit()
        keyguard(secure = true, locked = false)
    }

    private fun keyguard(secure: Boolean, locked: Boolean) {
        val k = shadowOf(c.getSystemService(KeyguardManager::class.java))
        k.setIsDeviceSecure(secure)
        k.setKeyguardLocked(locked)
    }

    private fun card(kind: CardKind, id: String = "val_1", title: String, body: String, canAllow: Boolean = false, dialog: Boolean = false) =
        InboxCard(
            id, kind, title, body, "", "", CardIcon.Info, if (kind == CardKind.HelperAsks) "s-1" else null, canAllow, "Allow", null, "", "",
            excerpt = if (kind == CardKind.HelperAsks) listOf("Run this command?", "cargo build --release") else emptyList(),
        )

    private val helper get() = card(CardKind.HelperAsks, title = "Claude needs your OK", body = "Run this command?\ncargo build --release", canAllow = true)
    private val approval get() = card(CardKind.Approval, title = "Delete 3 old backups?", body = "The nightly backup wants to remove the 3 oldest copies.", canAllow = true)
    private val harmless get() = card(CardKind.Approval, title = "Publish the weekly note?", body = "It goes to your list at 09:00.", canAllow = true)

    private fun text(n: Notification) = listOf(Notification.EXTRA_TITLE, Notification.EXTRA_TEXT, Notification.EXTRA_BIG_TEXT).joinToString(" | ") { n.extras.getCharSequence(it)?.toString().orEmpty() }
    private fun buttons(n: Notification) = n.actions.orEmpty().map { it.title.toString() }

    @Test fun generalWordsNameWhoAndWhatKindOnly() {
        assertEquals(NoticeWords.Words("Claude needs your OK", "Open HQ to see what it wants to do."), NoticeWords.general(c, helper))
        assertEquals(NoticeWords.Words("A task needs your OK", "Open HQ to see what it wants to do."), NoticeWords.general(c, approval))
        assertEquals(NoticeWords.Words("A helper is stuck", "Open HQ to see which one."), NoticeWords.general(c, card(CardKind.HelperStuck, title = "Claude keeps hitting it", body = "x")))
        assertEquals(NoticeWords.Words("Your server HQ needs your OK", "Open HQ to see what it wants to do."), NoticeWords.general(c, card(CardKind.Approval, id = ServerHq.ID_PREFIX + "7", title = "Delete it", body = "b")))
        assertEquals(NoticeWords.Words("An update from HQ", "Tap to read it."), NoticeWords.general(c, card(CardKind.Update, title = "Board meeting notes", body = "Draft agenda saved to Notebooks/Work.")))
        assertEquals(NoticeWords.Words("HQ answered", "Tap to read."), NoticeWords.answered(c))
    }

    @Test fun detailKeepsTheGeneralTitleAndNeverTheItemsOwn() {
        val d = NoticeWords.detail(c, approval)
        assertEquals("A task needs your OK", d.title)
        assertEquals("The nightly backup wants to remove the 3 oldest copies.", d.body)
        assertEquals("An update from HQ", NoticeWords.detail(c, card(CardKind.Update, title = "Board meeting notes", body = "Saved.")).title)
    }

    @Test fun noGeneralWordsShareFourWordsWithTheItem() {
        val fixtures = listOf(
            """{"id":"a1","kind":"action_needed","title":"Harness session 'claude in hq-oss' is waiting for you","description":"Session s-1 on mac-mini is blocked on a prompt or approval:\nDo you want to edit Chat.dc.html?\nRead it with x","state":"pending","metadata":{"extra":{"source_task":"session-supervisor"}},"actions":["approve","reject"]}""",
            """{"id":"a2","kind":"action_needed","title":"Delete 3 old backups?","description":"The nightly backup wants to remove the 3 oldest copies, from before Sep 21.","state":"pending","actions":["approve","reject"]}""",
            """{"id":"a3","kind":"value_item","title":"Backup finished","description":"2,860 notes saved to your Mac.","state":"pending","actions":["dismiss"]}""",
            """{"id":"a4","kind":"action_needed","title":"FR-12: Plan the quarterly review","description":"Ready for you to check the quarterly plan.","state":"pending","metadata":{"extra":{"source_task":"task_ready_for_review"}},"actions":[]}""",
        )
        for (json in fixtures) {
            val o = JSONObject(json)
            val shown = text(AlertPoster.build(c, InboxWords.card(AlertPoster.itemOf(o)), details = false, quiet = false))
            for (source in listOf(o.getString("title"), o.getString("description"))) assertFalse("$shown leaks $source", sharesRun(shown, source, 4))
        }
    }

    private fun sharesRun(shown: String, source: String, n: Int): Boolean {
        val s = source.lowercase().split(Regex("\\W+")).filter { it.isNotEmpty() }
        val hay = shown.lowercase().split(Regex("\\W+")).filter { it.isNotEmpty() }.joinToString(" ")
        return s.windowed(n).any { hay.contains(it.joinToString(" ")) }
    }

    @Test fun detailsAreOffUnlessTheOwnerTurnedThemOn() {
        assertFalse(AlertPoster.showDetails(c))
        c.getSharedPreferences("alerts", Context.MODE_PRIVATE).edit().putBoolean("allow_answers", true).commit()
        assertFalse(AlertPoster.showDetails(c))
        AlertPoster.setShowDetails(c, true)
        assertTrue(AlertPoster.showDetails(c))
    }

    @Test fun detailsNeedTheSwitchASecureLockAndAnUnlockedPhone() {
        assertFalse(AlertPoster.detailsNow(c))
        AlertPoster.setShowDetails(c, true)
        assertTrue(AlertPoster.detailsNow(c))
        keyguard(secure = true, locked = true)
        assertFalse(AlertPoster.detailsNow(c))
        keyguard(secure = false, locked = false)
        assertFalse(AlertPoster.detailsNow(c))
    }

    @Test fun detailsOffGiveGeneralWordsAndOpenOnly() {
        val n = AlertPoster.build(c, helper, details = false, quiet = false)
        assertEquals("Claude needs your OK | Open HQ to see what it wants to do. | Open HQ to see what it wants to do.", text(n))
        assertEquals(listOf("Open"), buttons(n))
    }

    @Test fun detailsOnAndUnlockedGiveDeclineAllowOnceOpenInThatOrder() {
        val n = AlertPoster.build(c, helper, details = true, quiet = false)
        assertTrue(text(n).contains("cargo build --release"))
        assertEquals(listOf("Decline", "Allow once", "Open"), buttons(n))
        assertTrue(n.actions[1].isAuthenticationRequired)
        assertFalse(n.actions[0].isAuthenticationRequired)
    }

    @Test fun aLosingRequestShowsDetailsButKeepsOpenOnly() {
        val n = AlertPoster.build(c, approval, details = true, quiet = false)
        assertTrue(text(n).contains("3 oldest copies"))
        assertEquals(listOf("Open"), buttons(n))
        assertEquals(listOf("Decline", "Allow once", "Open"), buttons(AlertPoster.build(c, harmless, details = true, quiet = false)))
    }

    @Test fun aLosingRequestNeverGetsAllowWithDetailsOff() {
        assertEquals(listOf("Open"), buttons(AlertPoster.build(c, approval, details = false, quiet = false)))
    }

    @Test fun aStuckHelperAndAnUpdateHaveNoAnswerButtons() {
        assertEquals(listOf("Open"), buttons(AlertPoster.build(c, card(CardKind.HelperStuck, title = "t", body = "b"), details = true, quiet = false)))
        assertTrue(AlertPoster.build(c, card(CardKind.Update, title = "t", body = "b"), details = true, quiet = false).actions.isNullOrEmpty())
    }

    @Test @Config(sdk = [30]) fun beforeAndroid12DetailsShowWithOpenOnly() {
        val n = AlertPoster.build(c, helper, details = true, quiet = false)
        assertTrue(text(n).contains("cargo build --release"))
        assertEquals(listOf("Open"), buttons(n))
    }

    @Test fun aHelperQuestionWithADialogOffersDeclineAlways() {
        val withDialog = helper.copy(canAllow = false, dialog = example.hq.spike.data.HqClient.AlertDialog("s-1", 1L, "command", "Run it?", "cargo build", null, false))
        assertEquals(listOf("Decline", "Open"), buttons(AlertPoster.build(c, withDialog, details = true, quiet = false)))
    }

    @Test fun everyNoticeCarriesAPublicVersionAndPrivateVisibility() {
        val needs = AlertPoster.build(c, helper, details = false, quiet = false)
        assertEquals(Notification.VISIBILITY_PRIVATE, needs.visibility)
        assertEquals("Something needs you | Unlock to see what. | ", text(needs.publicVersion).replace(" | $", " | "))
        val update = AlertPoster.build(c, card(CardKind.Update, title = "t", body = "b"), details = false, quiet = false)
        assertEquals("1 new update", update.publicVersion.extras.getCharSequence(Notification.EXTRA_TITLE).toString())
        assertEquals("Unlock to read it.", update.publicVersion.extras.getCharSequence(Notification.EXTRA_TEXT).toString())
    }

    @Test fun theSimpleNoticesKeepThePublicVersionToo() {
        AlertPoster.post(c, mapOf("v1" to item("v1", "Delete 3 old backups?")), listOf("v1", "v2", "v3"), 1)
        val folded = shadowOf(nm).allNotifications.first { text(it).startsWith("2 more waiting for you") }
        assertEquals("Open HQ to see them.", folded.extras.getCharSequence(Notification.EXTRA_TEXT).toString())
        assertNotNull(folded.publicVersion)
        assertEquals(Notification.VISIBILITY_PRIVATE, folded.visibility)
    }

    private fun item(id: String, title: String) = JSONObject(
        """{"id":"$id","kind":"action_needed","title":"$title","description":"The nightly backup wants to remove the 3 oldest copies.","state":"pending","actions":["approve","reject"]}""",
    )

    @Test fun theServerSignOutNamesNoAddress() {
        AlertPoster.serverSignedOut(c)
        val n = shadowOf(nm).allNotifications.single()
        assertEquals("Your server HQ signed this phone out | Its questions stop arriving here. Sign in again in Settings. | Its questions stop arriving here. Sign in again in Settings.", text(n))
        assertFalse(text(n).contains("hq.example.ts.net"))
        assertNotNull(n.publicVersion)
        assertFalse(text(n.publicVersion).contains("hq.example.ts.net"))
    }

    @Test fun lockingPostsTheGeneralWordsOverADetailedNotice() {
        AlertPoster.setShowDetails(c, true)
        val items = mapOf("v1" to item("v1", "Publish the weekly note?"))
        AlertPoster.post(c, items, listOf("v1"), 5)
        assertTrue(text(shadowOf(nm).allNotifications.single()).contains("3 oldest copies"))
        AlertPoster.repost(c, items, forceGeneral = true)
        val locked = shadowOf(nm).allNotifications.single()
        assertFalse(text(locked).contains("3 oldest copies"))
        assertEquals(listOf("Open"), buttons(locked))
        assertTrue(locked.flags and Notification.FLAG_ONLY_ALERT_ONCE != 0)
        AlertPoster.repost(c, items, forceGeneral = false)
        val back = shadowOf(nm).allNotifications.single()
        assertTrue(text(back).contains("3 oldest copies"))
        assertTrue(back.flags and Notification.FLAG_ONLY_ALERT_ONCE != 0)
    }

    @Test fun aNoticePostedWhileLockedIsGeneralFromTheStart() {
        AlertPoster.setShowDetails(c, true)
        keyguard(secure = true, locked = true)
        AlertPoster.post(c, mapOf("v1" to item("v1", "Publish the weekly note?")), listOf("v1"), 5)
        assertFalse(text(shadowOf(nm).allNotifications.single()).contains("3 oldest copies"))
    }

    @Test fun turningDetailsOffTakesThemOffTheShadeAtOnce() {
        AlertPoster.setShowDetails(c, true)
        val items = mapOf("v1" to item("v1", "Publish the weekly note?"))
        AlertPoster.post(c, items, listOf("v1"), 5)
        Alerts.track(items)
        AlertPoster.setShowDetails(c, false)
        assertFalse(text(shadowOf(nm).allNotifications.single()).contains("3 oldest copies"))
    }

    @Test fun withNoSecureLockDetailsNeverPost() {
        AlertPoster.setShowDetails(c, true)
        keyguard(secure = false, locked = false)
        AlertPoster.post(c, mapOf("v1" to item("v1", "Publish the weekly note?")), listOf("v1"), 5)
        val n = shadowOf(nm).allNotifications.single()
        assertFalse(text(n).contains("3 oldest copies"))
        assertEquals(listOf("Open"), buttons(n))
    }

    @Test fun theFiveChannelsAreMadeOnceWithTheBoardsImportance() {
        val ch = AlertPoster.channels(c).associateBy { it.id }
        assertEquals(listOf("needs", "updates", "app_updates", "downloads", "hub_min"), AlertPoster.channels(c).map { it.id })
        assertEquals(NotificationManager.IMPORTANCE_HIGH, ch.getValue("needs").importance)
        assertEquals(NotificationManager.IMPORTANCE_DEFAULT, ch.getValue("updates").importance)
        assertEquals(null, ch.getValue("updates").sound)
        assertEquals(NotificationManager.IMPORTANCE_LOW, ch.getValue("app_updates").importance)
        assertEquals(NotificationManager.IMPORTANCE_LOW, ch.getValue("downloads").importance)
        assertEquals(NotificationManager.IMPORTANCE_MIN, ch.getValue("hub_min").importance)
        assertEquals("HQ is running", ch.getValue("hub_min").name.toString())
    }

    @Test fun anUpgraderStartsWithDetailsOffAndSeesTheNoteOnce() {
        c.getSharedPreferences("alerts", Context.MODE_PRIVATE).edit().putString("seen", "a\nb").putBoolean("allow_answers", true).commit()
        AlertPoster.migrate(c)
        assertFalse(AlertPoster.showDetails(c))
        assertFalse(c.getSharedPreferences("alerts", Context.MODE_PRIVATE).contains("allow_answers"))
        assertTrue(AlertPoster.upgraderNote(c))
        AlertPoster.clearUpgraderNote(c)
        assertFalse(AlertPoster.upgraderNote(c))
    }

    @Test fun aFreshInstallGetsNoUpgraderNote() {
        AlertPoster.migrate(c)
        assertFalse(AlertPoster.upgraderNote(c))
    }

    @Test fun aFailedAnswerReadsTheCatalogueWordsNotTheRetiredOnes() {
        assertEquals("Couldn't send your answer.", c.getString(R.string.fail_send_answer_title))
        assertEquals("HQ didn't answer in time. Try again.", c.getString(R.string.fail_send_answer_no_reply_body))
        assertFalse(c.getString(R.string.fail_send_answer_no_reply_body).contains("didn't reply"))
    }
}

package example.hq.spike

import android.app.KeyguardManager
import android.app.Notification
import android.content.Context
import androidx.activity.ComponentActivity
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.core.app.ApplicationProvider
import example.hq.spike.data.SandboxGrant
import example.hq.spike.ui.helpers.GrantsSheet
import example.hq.spike.ui.helpers.HelperOpen
import example.hq.spike.ui.helpers.HelperPage
import example.hq.spike.ui.helpers.HelpersActions
import example.hq.spike.ui.inbox.CardIcon
import example.hq.spike.ui.inbox.CardKind
import example.hq.spike.ui.inbox.InboxActions
import example.hq.spike.ui.inbox.InboxCard
import example.hq.spike.ui.inbox.InboxScreen
import example.hq.spike.ui.inbox.InboxUi
import example.hq.spike.ui.inbox.InboxWords
import example.hq.spike.ui.inbox.ParsedSandboxGrant
import example.hq.spike.ui.theme.HqTheme
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w390dp-h900dp-xxhdpi")
class SandboxGrantNoticeTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()
    private val c: Context = ApplicationProvider.getApplicationContext()

    @Before fun unlock() {
        val km = shadowOf(c.getSystemService(KeyguardManager::class.java))
        km.setIsDeviceSecure(true)
        km.setKeyguardLocked(false)
    }

    @Test fun sandboxGrantNotificationIsConservativeOpenOnlyAndOpensInboxItem() {
        val raw = JSONObject(
            """{
              "id":"val_sg_1",
              "kind":"action_needed",
              "title":"A helper wants to reach example.org:443",
              "description":"Helper session hs-7 asked to reach example.org:443. HQ could not approve it by itself: it is outside the access this task was given. Approving applies to this session only and lasts up to a day; the session restarts to pick it up. The helper's own words, not HQ's: \"Need registry metadata\". Dismiss to refuse.",
              "state":"pending",
              "created_at":"2026-11-20T08:00:00+00:00",
              "metadata":{"extra":{"source_task":"sandbox-grants"}},
              "actions":["approve","reject"]
            }""",
        )
        val card = InboxWords.card(AlertPoster.itemOf(raw))
        assertEquals(CardKind.SandboxGrant, card.kind)
        assertEquals("hq://inbox/val_sg_1", AlertPoster.linkFor(card))

        val general = AlertPoster.build(c, card, details = false, quiet = false)
        assertEquals("Access request", general.extras.getCharSequence(Notification.EXTRA_TITLE).toString())
        assertFalse(general.extras.getCharSequence(Notification.EXTRA_TEXT).toString().contains("example.org"))
        assertEquals(listOf("Open"), general.actions.orEmpty().map { it.title.toString() })

        // Conservatively keeps Open only even when unlocked with details on, since notifications carry no risk class.
        val detailed = AlertPoster.build(c, card, details = true, quiet = false)
        assertEquals(listOf("Open"), detailed.actions.orEmpty().map { it.title.toString() })
    }

    @Test fun inboxCardRendersGrantDetailsWithAllowAndDeclineOnly() {
        var allowed = 0
        var declined = 0
        val card = InboxCard(
            id = "val_sg_2",
            kind = CardKind.SandboxGrant,
            title = "A helper wants to write to /home/user/.gradle",
            body = "/home/user/.gradle",
            where = "A helper",
            createdAt = "2026-11-20T08:00:00+00:00",
            icon = CardIcon.Info,
            sessionId = "hs-1",
            canAllow = true,
            allowLabel = "Allow",
            link = null,
            asked = "A helper",
            state = "pending",
            sandboxGrant = ParsedSandboxGrant(
                sessionId = "hs-1",
                taskId = "FR-42",
                capability = "fs_write",
                resource = "/home/user/.gradle",
                helperReason = "Need the Gradle cache to build",
            ),
        )
        rule.setContent {
            HqTheme {
                InboxScreen(
                    ui = InboxUi(loaded = true, needs = listOf(card)),
                    actions = InboxActions(onAllow = { allowed++ }, onNotNow = { declined++ }),
                    onTakeLook = {},
                )
            }
        }
        rule.onNodeWithText("A helper wants to write to /home/user/.gradle").assertIsDisplayed()
        rule.onNodeWithText("Write access").assertIsDisplayed()
        rule.onNodeWithText("/home/user/.gradle").assertIsDisplayed()
        rule.onNodeWithText("FR-42").assertIsDisplayed()
        rule.onNodeWithText("Expires in a day").assertIsDisplayed()
        rule.onNodeWithText("Helper's own words, not HQ's").assertIsDisplayed()
        rule.onNodeWithText("Need the Gradle cache to build").assertIsDisplayed()
        rule.onNodeWithText("Allow").performClick()
        rule.onNodeWithText("Decline").performClick()
        assertEquals(1, allowed)
        assertEquals(1, declined)
    }

    @Test fun grantsSheetListsActiveWithRevokeAndMutedHistoryWithDecisionNotes() {
        val revokedIds = mutableListOf<String>()
        val open = HelperOpen(
            id = "hs-1",
            page = HelperPage.Detail,
            grantsOpen = true,
            grants = listOf(
                SandboxGrant("p-1", "hs-1", "FR-42", "local", "/home/user/app", "profile", "mobile-dev", "active", "owner", "", "", 1L, null, null, null, null),
                SandboxGrant("g-1", "hs-1", "FR-42", "local", "/home/user/app", "fs_write", "/home/user/.gradle", "active", "policy", "", "", 2L, 2L, 2L, 9L, null),
                SandboxGrant("g-2", "hs-1", "FR-42", "local", "/home/user/app", "egress", "example.org:443", "revoked", "owner", "", "no longer needed", 3L, 3L, null, null, 4L),
                SandboxGrant("g-3", "hs-1", "FR-42", "local", "/home/user/app", "fs_read", "/home/user/.ssh", "refused", "system", "", "credential directory", 4L, 4L, null, null, null),
                SandboxGrant("g-4", "hs-1", "FR-42", "local", "/home/user/app", "egress", "example.org:80", "declined", "owner", "", "use https instead", 5L, 5L, null, null, null),
                SandboxGrant("g-5", "hs-1", "FR-42", "local", "/home/user/app", "loopback", "5037", "expired", "policy", "", "24h ttl elapsed", 6L, 6L, null, 7L, null),
            ),
        )
        rule.setContent {
            HqTheme {
                GrantsSheet(
                    open = open,
                    actions = HelpersActions(onGrantRevoke = { revokedIds += it }),
                )
            }
        }
        rule.onNodeWithText("HQ allowed /home/user/.gradle").assertIsDisplayed()
        rule.onNodeWithText("Revoke").performClick()
        assertEquals(listOf("g-1"), revokedIds)

        rule.onNodeWithText("Revoked: example.org:443").assertIsDisplayed()
        rule.onNodeWithText("no longer needed").assertIsDisplayed()
        rule.onNodeWithText("Refused: /home/user/.ssh").assertIsDisplayed()
        rule.onNodeWithText("credential directory").assertIsDisplayed()
        rule.onNodeWithText("Declined: example.org:80").assertIsDisplayed()
        rule.onNodeWithText("use https instead").assertIsDisplayed()
        rule.onNodeWithText("Expired: 5037").assertIsDisplayed()
        rule.onNodeWithText("24h ttl elapsed").assertIsDisplayed()
    }
}

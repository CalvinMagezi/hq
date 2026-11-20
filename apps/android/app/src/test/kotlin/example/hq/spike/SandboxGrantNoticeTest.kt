package example.hq.spike

import android.app.KeyguardManager
import android.app.Notification
import android.content.Context
import androidx.activity.ComponentActivity
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.test.core.app.ApplicationProvider
import example.hq.spike.data.SandboxGrant
import example.hq.spike.data.SandboxGrantProfile
import example.hq.spike.data.SandboxGrantsSummary
import example.hq.spike.data.SandboxSummaryResponse
import example.hq.spike.ui.helpers.GrantsSheet
import example.hq.spike.ui.helpers.HelperOpen
import example.hq.spike.ui.helpers.HelperPage
import example.hq.spike.ui.helpers.HelpersActions
import example.hq.spike.ui.inbox.Answer
import example.hq.spike.ui.inbox.CardIcon
import example.hq.spike.ui.inbox.CardKind
import example.hq.spike.ui.inbox.InboxActions
import example.hq.spike.ui.inbox.InboxCard
import example.hq.spike.ui.inbox.InboxScreen
import example.hq.spike.ui.inbox.InboxUi
import example.hq.spike.ui.inbox.InboxWords
import example.hq.spike.ui.inbox.ParsedSandboxGrant
import example.hq.spike.ui.inbox.SandboxGrantAnswer
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

    @Test fun inboxCardRendersStandingChoicesStarterContentsAndServerError() {
        val answers = mutableListOf<SandboxGrantAnswer>()
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
                repo = "/work/app",
                capability = "fs_write",
                resource = "/home/user/.gradle",
                helperReason = "Need the Gradle cache to build",
                suggestedStarter = "mobile-dev",
            ),
        )
        val starters = listOf(
            SandboxGrantProfile(
                name = "mobile-dev",
                writable = listOf("~/.gradle", "~/.pub-cache"),
                readable = listOf("~/Library/Android/sdk"),
                loopbackPorts = listOf(5037),
                egress = listOf("example.org:443"),
            ),
        )
        rule.setContent {
            HqTheme {
                InboxScreen(
                    ui = InboxUi(
                        loaded = true,
                        needs = listOf(card),
                        sandboxStarters = starters,
                        answering = mapOf("val_sg_2" to Answer.Failed("this request has no repository")),
                    ),
                    actions = InboxActions(onSandboxGrantAnswer = { _, ans -> answers += ans }),
                    onTakeLook = {},
                )
            }
        }
        rule.onNodeWithText("A helper wants to write to /home/user/.gradle").assertIsDisplayed()
        rule.onNodeWithText("Write access").assertIsDisplayed()
        rule.onNodeWithText("/home/user/.gradle").assertIsDisplayed()
        rule.onNodeWithText("FR-42").assertIsDisplayed()
        rule.onNodeWithText("/work/app").assertIsDisplayed()
        rule.onNodeWithText("Expires in a day").assertIsDisplayed()
        rule.onNodeWithText("Helper's own words, not HQ's").assertIsDisplayed()
        rule.onNodeWithText("Need the Gradle cache to build").assertIsDisplayed()
        rule.onNodeWithText("this request has no repository").assertIsDisplayed()

        rule.onNodeWithText("Allow once for this helper").performClick()
        rule.onNodeWithText("Allow for every helper on this task").performClick()
        rule.onNodeWithText("Allow for this repository").performClick()
        rule.onNodeWithText("Allow mobile development tools for this repository").performClick()
        rule.onNodeWithText("Includes mobile development tools (Gradle, Flutter, Android SDK, their download sites):").assertIsDisplayed()
        rule.onNodeWithText("Write: ~/.gradle, ~/.pub-cache").assertIsDisplayed()
        rule.onNodeWithText("Read: ~/Library/Android/sdk").assertIsDisplayed()
        rule.onNodeWithText("Local ports: 5037").assertIsDisplayed()
        rule.onNodeWithText("Network: example.org:443").assertIsDisplayed()
        rule.onNodeWithText("Decline").performClick()

        assertEquals(
            listOf(
                SandboxGrantAnswer.AllowOnce,
                SandboxGrantAnswer.AllowTask,
                SandboxGrantAnswer.AllowRepo,
                SandboxGrantAnswer.AllowStarter("mobile-dev"),
                SandboxGrantAnswer.Decline,
            ),
            answers,
        )
    }

    @Test fun inboxCardHidesTaskRepoAndStarterChoicesWhenAbsent() {
        val card = InboxCard(
            id = "val_sg_3",
            kind = CardKind.SandboxGrant,
            title = "A helper wants to reach example.org:443",
            body = "example.org:443",
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
                taskId = null,
                repo = "",
                capability = "egress",
                resource = "example.org:443",
                helperReason = "Need docs",
                suggestedStarter = "mobile-dev",
            ),
        )
        rule.setContent {
            HqTheme {
                InboxScreen(
                    ui = InboxUi(loaded = true, needs = listOf(card)),
                    actions = InboxActions(),
                    onTakeLook = {},
                )
            }
        }
        rule.onNodeWithText("Allow once for this helper").assertIsDisplayed()
        rule.onNodeWithText("Decline").assertIsDisplayed()
        rule.onNodeWithText("Allow for every helper on this task").assertDoesNotExist()
        rule.onNodeWithText("Allow for this repository").assertDoesNotExist()
        rule.onNodeWithText("Allow mobile development tools for this repository").assertDoesNotExist()
    }

    @Test fun grantsSheetListsSummaryStandingRulesActiveAndHistoryWithRevoke() {
        val revokedIds = mutableListOf<String>()
        val open = HelperOpen(
            id = "hs-1",
            page = HelperPage.Detail,
            grantsOpen = true,
            grantsSummary = SandboxSummaryResponse(
                days = 7,
                summary = SandboxGrantsSummary(
                    approvedByPolicy = 5,
                    approvedByOwner = 2,
                    askedOwner = 3,
                    declined = 1,
                    refused = 1,
                    standingRules = 1,
                ),
            ),
            standingRules = listOf(
                SandboxGrant("r-1", "repo:/work/app", null, "local", "/work/app", "fs_write", "~/.gradle", "active", "owner", "", "mobile-dev starter", 1L, 1L, null, null, null, scope = "repo"),
            ),
            grants = listOf(
                SandboxGrant("p-1", "hs-1", "FR-42", "local", "/work/app", "profile", "mobile-dev", "active", "owner", "", "", 1L, null, null, null, null),
                SandboxGrant("g-1", "hs-1", "FR-42", "local", "/work/app", "fs_write", "/home/user/.gradle", "active", "policy", "", "", 2L, 2L, 2L, 9L, null),
                SandboxGrant("g-2", "hs-1", "FR-42", "local", "/work/app", "egress", "example.org:443", "revoked", "owner", "", "no longer needed", 3L, 3L, null, null, 4L),
                SandboxGrant("g-3", "hs-1", "FR-42", "local", "/work/app", "fs_read", "/home/user/.ssh", "refused", "system", "", "credential directory", 4L, 4L, null, null, null),
                SandboxGrant("g-4", "hs-1", "FR-42", "local", "/work/app", "egress", "example.org:80", "declined", "owner", "", "use https instead", 5L, 5L, null, null, null),
                SandboxGrant("g-5", "hs-1", "FR-42", "local", "/work/app", "loopback", "5037", "expired", "policy", "", "24h ttl elapsed", 6L, 6L, null, 7L, null),
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
        rule.onNodeWithText("In the last 7 days HQ allowed 5 requests for you, asked you 3, and refused 1.").assertIsDisplayed()
        rule.onNodeWithText("Standing rules").assertIsDisplayed()
        rule.onNodeWithText("Every helper in /work/app may write to ~/.gradle").assertIsDisplayed()
        rule.onNodeWithText("mobile-dev starter").assertIsDisplayed()

        rule.onNodeWithText("HQ allowed /home/user/.gradle").performScrollTo().assertIsDisplayed()
        rule.onNodeWithText("Revoked: example.org:443").performScrollTo().assertIsDisplayed()
        rule.onNodeWithText("no longer needed").assertIsDisplayed()
        rule.onNodeWithText("Refused: /home/user/.ssh").performScrollTo().assertIsDisplayed()
        rule.onNodeWithText("credential directory").assertIsDisplayed()
        rule.onNodeWithText("Declined: example.org:80").performScrollTo().assertIsDisplayed()
        rule.onNodeWithText("use https instead").assertIsDisplayed()
        rule.onNodeWithText("Expired: 5037").performScrollTo().assertIsDisplayed()
        rule.onNodeWithText("24h ttl elapsed").assertIsDisplayed()
    }

    @Test fun grantsSheetRevokesStandingRule() {
        val revokedIds = mutableListOf<String>()
        val open = HelperOpen(
            id = "hs-1",
            page = HelperPage.Detail,
            grantsOpen = true,
            standingRules = listOf(
                SandboxGrant("r-1", "task:FR-42", "FR-42", "local", "/work/app", "egress", "example.org:443", "active", "owner", "", "", 1L, 1L, null, null, null, scope = "task"),
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
        rule.onNodeWithText("Every helper on task FR-42 may reach example.org:443").assertIsDisplayed()
        rule.onNodeWithText("Revoke").performClick()
        assertEquals(listOf("r-1"), revokedIds)
    }
}

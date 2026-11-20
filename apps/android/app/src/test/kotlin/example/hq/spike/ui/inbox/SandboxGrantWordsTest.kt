package example.hq.spike.ui.inbox

import example.hq.spike.R
import example.hq.spike.data.HqClient.InboxItem
import example.hq.spike.data.SANDBOX_GRANTS_SOURCE
import example.hq.spike.data.SandboxGrant
import example.hq.spike.ui.helpers.visibleSessionGrants
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class SandboxGrantWordsTest {
    private fun item(title: String, description: String) = InboxItem(
        id = "val_sg_1",
        kind = "action_needed",
        title = title,
        description = description,
        state = "pending",
        createdAt = "2026-11-20T08:00:00+00:00",
        link = null,
        sourceTask = SANDBOX_GRANTS_SOURCE,
        valKind = "action_needed",
        actions = listOf("approve", "reject"),
    )

    private fun fileForOwnerDesc(session: String, what: String, reason: String) =
        "Helper session $session asked to $what. HQ could not approve it by itself: it is outside the access this task was given. " +
            "Approving applies to this session only and lasts up to a day; the session restarts to pick it up. " +
            "The helper's own words, not HQ's: \"$reason\". Dismiss to refuse."

    @Test fun parsesAllFourCapabilityDescriptionsFromFileForOwner() {
        val write = InboxWords.card(
            item(
                "A helper wants to write to /home/user/.gradle",
                fileForOwnerDesc("hs-1", "write to /home/user/.gradle", "Gradle needs its cache"),
            ),
        )
        assertEquals(CardKind.SandboxGrant, write.kind)
        assertEquals("A helper wants to write to /home/user/.gradle", write.title)
        assertEquals("hs-1", write.sessionId)
        assertTrue(write.canAllow)
        val wg = write.sandboxGrant
        assertNotNull(wg)
        assertEquals("fs_write", wg!!.capability)
        assertEquals("/home/user/.gradle", wg.resource)
        assertEquals("Gradle needs its cache", wg.helperReason)
        assertNull(wg.taskId)

        val read = SandboxGrantWords.parse(
            item("A helper wants to read /home/user/sdk", fileForOwnerDesc("hs-2", "read /home/user/sdk", "Read SDK headers")),
        )
        assertEquals("fs_read", read.capability)
        assertEquals("/home/user/sdk", read.resource)

        val loopback = SandboxGrantWords.parse(
            item("A helper wants to connect to local port 5037", fileForOwnerDesc("hs-3", "connect to local port 5037", "Connect to test server")),
        )
        assertEquals("loopback", loopback.capability)
        assertEquals("5037", loopback.resource)

        val egress = SandboxGrantWords.parse(
            item("A helper wants to reach example.org:443", fileForOwnerDesc("hs-4", "reach example.org:443", "Download package metadata")),
        )
        assertEquals("egress", egress.capability)
        assertEquals("example.org:443", egress.resource)
    }

    @Test fun enrichesPendingGrantWithTaskIdAndIgnoresProfiles() {
        val parsed = SandboxGrantWords.parse(
            item("A helper wants to write to /home/user/.gradle", fileForOwnerDesc("hs-1", "write to /home/user/.gradle", "Gradle cache")),
        )
        val grants = listOf(
            grant("p-1", "hs-1", "FR-99", "profile", "mobile-dev", "active"),
            grant("g-2", "hs-1", "FR-42", "fs_write", "/home/user/.gradle", "pending", reason = "Exact helper reason"),
        )
        val enriched = SandboxGrantWords.enrich(parsed, grants)
        assertEquals("FR-42", enriched.taskId)
        assertEquals("Exact helper reason", enriched.helperReason)
    }

    @Test fun clipsLongHelperReasonsAt300Chars() {
        val long = "a".repeat(320)
        val clipped = SandboxGrantWords.clipReason("  $long  ")
        assertEquals(303, clipped.length)
        assertTrue(clipped.endsWith("..."))
    }

    @Test fun mapsModesAndStatesToCatalogueStrings() {
        assertEquals(R.string.grants_mode_fs_write, SandboxGrantWords.modeRes("fs_write"))
        assertEquals(R.string.grants_mode_fs_read, SandboxGrantWords.modeRes("fs_read"))
        assertEquals(R.string.grants_mode_loopback, SandboxGrantWords.modeRes("loopback"))
        assertEquals(R.string.grants_mode_egress, SandboxGrantWords.modeRes("egress"))
        assertEquals(R.string.grants_mode_other, SandboxGrantWords.modeRes("unknown"))

        assertEquals(R.string.grants_state_revoked, SandboxGrantWords.stateRes("revoked"))
        assertEquals(R.string.grants_state_expired, SandboxGrantWords.stateRes("expired"))
        assertEquals(R.string.grants_state_refused, SandboxGrantWords.stateRes("refused"))
        assertEquals(R.string.grants_state_declined, SandboxGrantWords.stateRes("declined"))
    }

    @Test fun visibleSessionGrantsExcludesProfilesAndPendingRows() {
        val all = listOf(
            grant("p-1", "hs-1", "FR-1", "profile", "mobile-dev", "active"),
            grant("g-1", "hs-1", "FR-1", "fs_write", "/home/user/.gradle", "active"),
            grant("g-2", "hs-1", "FR-1", "egress", "example.org:443", "pending"),
            grant("g-3", "hs-1", "FR-1", "fs_read", "/home/user/.ssh", "refused", note = "credential path"),
            grant("g-4", "hs-1", "FR-1", "egress", "example.org:80", "declined", note = "owner declined"),
        )
        assertEquals(listOf("g-1", "g-3", "g-4"), visibleSessionGrants(all).map { it.id })
    }

    private fun grant(
        id: String,
        session: String,
        task: String?,
        capability: String,
        resource: String,
        state: String,
        reason: String = "",
        note: String = "",
    ) = SandboxGrant(
        id = id,
        sessionId = session,
        taskId = task,
        host = "local",
        repo = "/home/user/app",
        capability = capability,
        resource = resource,
        state = state,
        decidedBy = "policy",
        helperReason = reason,
        decisionNote = note,
        requestedAt = 1700000000L,
        decidedAt = null,
        appliedAt = null,
        expiresAt = null,
        revokedAt = null,
    )
}

package example.hq.spike.data

import java.net.InetAddress
import java.net.ServerSocket
import java.util.concurrent.CopyOnWriteArrayList
import kotlinx.coroutines.runBlocking
import org.json.JSONObject
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Before
import org.junit.Test

class HqClientGrantsTest {
    private lateinit var server: ServerSocket
    private val requestLines = CopyOnWriteArrayList<String>()
    private val requestBodies = CopyOnWriteArrayList<String>()
    private var answer: (String, String) -> Pair<Int, String> = { _, _ -> 200 to "{}" }

    @Before fun start() {
        server = ServerSocket(0, 50, InetAddress.getByName("127.0.0.1"))
        Thread {
            while (!server.isClosed) {
                try {
                    server.accept().use { socket ->
                        val input = socket.getInputStream().bufferedReader()
                        val line = input.readLine() ?: return@use
                        requestLines += line
                        var contentLength = 0
                        while (true) {
                            val header = input.readLine().orEmpty()
                            if (header.isEmpty()) break
                            if (header.startsWith("Content-Length:", ignoreCase = true)) {
                                contentLength = header.substringAfter(':').trim().toIntOrNull() ?: 0
                            }
                        }
                        val bodyChars = CharArray(contentLength)
                        var read = 0
                        while (read < contentLength) {
                            val n = input.read(bodyChars, read, contentLength - read)
                            if (n < 0) break
                            read += n
                        }
                        val reqBody = String(bodyChars, 0, read)
                        requestBodies += reqBody
                        val (code, body) = answer(line, reqBody)
                        val bytes = body.toByteArray()
                        socket.getOutputStream().apply {
                            write("HTTP/1.1 $code X\r\nContent-Type: application/json\r\nContent-Length: ${bytes.size}\r\nConnection: close\r\n\r\n".toByteArray())
                            write(bytes)
                            flush()
                        }
                    }
                } catch (e: java.io.IOException) {
                    // Server closed at the end of the test.
                }
            }
        }.apply { isDaemon = true }.start()
    }

    @After fun stop() = server.close()

    private fun client() = HqClient({ "t-owner" }, "http://127.0.0.1:${server.localPort}")

    @Test fun listsAllGrantsWithScopeAndSuggestedStarterAndFiltersBySession() = runBlocking {
        val payload = """
            {"grants":[
              {"id":"g-1","session_id":"hs 1","task_id":"FR-42","host":"local","repo":"/work/app","capability":"fs_write","resource":"/home/user/.gradle","state":"pending","decided_by":"policy","helper_reason":"need cache","decision_note":"","requested_at":1700000000,"decided_at":null,"applied_at":null,"expires_at":1700086400,"revoked_at":null,"scope":"session","suggested_starter":"mobile-dev"},
              {"id":"g-2","session_id":"hs 1","task_id":null,"host":"local","repo":"/work/app","capability":"egress","resource":"example.org:443","state":"refused","decided_by":"system","helper_reason":"fetch docs","decision_note":"outside profile","requested_at":1700000100,"decided_at":null,"applied_at":null,"expires_at":null,"revoked_at":null,"scope":"session","suggested_starter":null}
            ]}
        """.trimIndent()
        answer = { _, _ -> 200 to payload }

        val all = client().sandboxGrants()
        assertEquals("GET /api/sandbox-grants HTTP/1.1", requestLines[0])
        assertEquals(2, all.size)
        assertEquals("g-1", all[0].id)
        assertEquals("FR-42", all[0].taskId)
        assertEquals("/work/app", all[0].repo)
        assertEquals("/home/user/.gradle", all[0].resource)
        assertEquals("session", all[0].scope)
        assertEquals("mobile-dev", all[0].suggestedStarter)
        assertTrue(all[0].isPending && all[0].isResource && !all[0].isStandingRule)
        assertNull(all[0].revokedAt)
        assertEquals(1700086400L, all[0].expiresAt)

        assertNull(all[1].taskId)
        assertNull(all[1].suggestedStarter)
        assertEquals("example.org:443", all[1].resource)
        assertEquals("outside profile", all[1].decisionNote)
        assertFalse(all[1].isActive)

        client().sandboxGrants("hs 1")
        assertEquals("GET /api/sandbox-grants?session=hs+1 HTTP/1.1", requestLines[1])
    }

    @Test fun listsStandingRulesStartersAndSummary() = runBlocking {
        val rulesJson = """
            {"rules":[
              {"id":"r-1","session_id":"repo:/work/app","task_id":null,"host":"local","repo":"/work/app","capability":"fs_write","resource":"~/.gradle","state":"active","decided_by":"owner","helper_reason":"","decision_note":"mobile-dev starter","requested_at":1700000000,"decided_at":1700000001,"applied_at":null,"expires_at":null,"revoked_at":null,"scope":"repo"},
              {"id":"r-2","session_id":"task:FR-42","task_id":"FR-42","host":"local","repo":"/work/app","capability":"egress","resource":"example.org:443","state":"active","decided_by":"owner","helper_reason":"","decision_note":"","requested_at":1700000002,"decided_at":1700000003,"applied_at":null,"expires_at":null,"revoked_at":null,"scope":"task"}
            ]}
        """.trimIndent()
        answer = { _, _ -> 200 to rulesJson }
        val rules = client().sandboxGrantRules()
        assertEquals("GET /api/sandbox-grants/rules HTTP/1.1", requestLines[0])
        assertEquals(2, rules.size)
        assertTrue(rules[0].isStandingRule)
        assertEquals("repo", rules[0].scope)
        assertEquals("~/.gradle", rules[0].resource)
        assertTrue(rules[1].isStandingRule)
        assertEquals("task", rules[1].scope)
        assertEquals("FR-42", rules[1].taskId)

        val startersJson = """
            {"starters":[
              {"name":"mobile-dev","writable":["~/.gradle","~/.pub-cache"],"readable":["~/Library/Android/sdk"],"loopback_ports":[5037],"egress":["example.org:443"]}
            ]}
        """.trimIndent()
        answer = { _, _ -> 200 to startersJson }
        val starters = client().sandboxGrantStarters()
        assertEquals("GET /api/sandbox-grants/starters HTTP/1.1", requestLines[1])
        assertEquals(1, starters.size)
        assertEquals("mobile-dev", starters[0].name)
        assertEquals(listOf("~/.gradle", "~/.pub-cache"), starters[0].writable)
        assertEquals(listOf("~/Library/Android/sdk"), starters[0].readable)
        assertEquals(listOf(5037), starters[0].loopbackPorts)
        assertEquals(listOf("example.org:443"), starters[0].egress)

        val summaryJson = """
            {"days":7,"summary":{"approved_by_policy":5,"approved_by_owner":2,"asked_owner":3,"declined":1,"refused":1,"standing_rules":4}}
        """.trimIndent()
        answer = { _, _ -> 200 to summaryJson }
        val summary = client().sandboxGrantSummary(7)
        assertEquals("GET /api/sandbox-grants/summary?days=7 HTTP/1.1", requestLines[2])
        assertEquals(7, summary.days)
        assertEquals(5, summary.summary.approvedByPolicy)
        assertEquals(2, summary.summary.approvedByOwner)
        assertEquals(3, summary.summary.askedOwner)
        assertEquals(1, summary.summary.declined)
        assertEquals(1, summary.summary.refused)
        assertEquals(4, summary.summary.standingRules)
    }

    @Test fun answersNotificationWithStarterAndSurfaces400ErrorMessage() = runBlocking {
        answer = { _, _ -> 200 to """{"success":true,"new_state":"approved","message":"ok"}""" }
        client().answerNotification("val_sg_1", "allow_starter", "mobile-dev")
        assertEquals("POST /api/notifications/val_sg_1/action HTTP/1.1", requestLines[0])
        val sent = JSONObject(requestBodies[0])
        assertEquals("allow_starter", sent.getString("action"))
        assertEquals("mobile-dev", sent.getString("starter"))

        answer = { _, _ -> 400 to """{"error":"this request has no repository to attach a rule to"}""" }
        try {
            client().answerNotification("val_sg_1", "allow_repo")
            fail("400 must throw HqUnavailable")
        } catch (e: HqUnavailable) {
            assertEquals(400, e.status)
            assertEquals("this request has no repository to attach a rule to", e.serverMessage)
        }
    }

    @Test fun revokesGrantAndSurfaces404() = runBlocking {
        answer = { _, _ -> 200 to """{"revoked":"g-1"}""" }
        assertEquals("g-1", client().revokeSandboxGrant("g-1"))
        assertEquals("POST /api/sandbox-grants/g-1/revoke HTTP/1.1", requestLines.single())

        answer = { _, _ -> 404 to """{"error":"no live grant g-1"}""" }
        try {
            client().revokeSandboxGrant("g-1")
            fail("404 must throw HqUnavailable")
        } catch (e: HqUnavailable) {
            assertEquals(404, e.status)
            assertEquals("no live grant g-1", e.serverMessage)
        }
    }

    @Test fun listsProfilesAndBindsProfileForTaskOrSession() = runBlocking {
        val profilesJson = """
            {"profiles":[
              {"name":"mobile-dev","writable":["/home/user/.gradle"],"readable":["/home/user/sdk"],"loopback_ports":[5037],"egress":["example.org:443"]}
            ]}
        """.trimIndent()
        answer = { _, _ -> 200 to profilesJson }
        val profiles = client().sandboxGrantProfiles()
        assertEquals("GET /api/sandbox-grants/profiles HTTP/1.1", requestLines[0])
        assertEquals(1, profiles.size)
        assertEquals("mobile-dev", profiles[0].name)
        assertEquals(listOf("/home/user/.gradle"), profiles[0].writable)
        assertEquals(listOf("/home/user/sdk"), profiles[0].readable)
        assertEquals(listOf(5037), profiles[0].loopbackPorts)
        assertEquals(listOf("example.org:443"), profiles[0].egress)

        answer = { _, _ -> 200 to """{"bound":"mobile-dev"}""" }
        assertEquals("mobile-dev", client().bindSandboxProfile("task", "tk-1", "mobile-dev"))
        assertEquals("POST /api/sandbox-grants/profile HTTP/1.1", requestLines[1])
        val sent = JSONObject(requestBodies[1])
        assertEquals("task", sent.getString("scope"))
        assertEquals("tk-1", sent.getString("id"))
        assertEquals("mobile-dev", sent.getString("name"))
    }
}

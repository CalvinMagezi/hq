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

    @Test fun listsAllGrantsAndFiltersBySession() = runBlocking {
        val payload = """
            {"grants":[
              {"id":"g-1","session_id":"hs 1","task_id":"FR-42","host":"local","repo":"/home/user/app","capability":"fs_write","resource":"/home/user/.gradle","state":"active","decided_by":"policy","helper_reason":"need cache","decision_note":"","requested_at":1700000000,"decided_at":1700000001,"applied_at":1700000002,"expires_at":1700086400,"revoked_at":null},
              {"id":"g-2","session_id":"hs 1","task_id":null,"host":"local","repo":"/home/user/app","capability":"egress","resource":"example.org:443","state":"refused","decided_by":"system","helper_reason":"fetch docs","decision_note":"outside profile","requested_at":1700000100,"decided_at":null,"applied_at":null,"expires_at":null,"revoked_at":null}
            ]}
        """.trimIndent()
        answer = { _, _ -> 200 to payload }

        val all = client().sandboxGrants()
        assertEquals("GET /api/sandbox-grants HTTP/1.1", requestLines[0])
        assertEquals(2, all.size)
        assertEquals("g-1", all[0].id)
        assertEquals("FR-42", all[0].taskId)
        assertEquals("/home/user/.gradle", all[0].resource)
        assertTrue(all[0].isActive && all[0].isResource)
        assertNull(all[0].revokedAt)
        assertEquals(1700086400L, all[0].expiresAt)

        assertNull(all[1].taskId)
        assertEquals("example.org:443", all[1].resource)
        assertEquals("outside profile", all[1].decisionNote)
        assertFalse(all[1].isActive)

        client().sandboxGrants("hs 1")
        assertEquals("GET /api/sandbox-grants?session=hs+1 HTTP/1.1", requestLines[1])
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

package example.hq.spike.data

import java.net.InetAddress
import java.net.ServerSocket
import java.util.concurrent.CopyOnWriteArrayList
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Before
import org.junit.Test

/** The client against a real HTTP server on loopback, so status codes, paging and token handling are the real thing. */
class HqClientTest {
    private lateinit var server: ServerSocket
    private val requests = CopyOnWriteArrayList<String>()
    private var answer: (String) -> Pair<Int, String> = { 200 to """{"tasks":[],"has_more":false}""" }

    /** A one-thread HTTP/1.1 responder: reads a request's head, answers with Content-Length and closes. */
    @Before fun start() {
        server = ServerSocket(0, 50, InetAddress.getByName("127.0.0.1"))
        Thread {
            while (!server.isClosed) {
                try {
                    server.accept().use { socket ->
                        val input = socket.getInputStream().bufferedReader()
                        val line = input.readLine() ?: return@use
                        while (input.readLine().orEmpty().isNotEmpty()) { /* skip the headers; none of these requests has a body to read */ }
                        val query = line.split(' ').getOrElse(1) { "" }.substringAfter('?', "")
                        requests += query
                        val (code, body) = answer(query)
                        val bytes = body.toByteArray()
                        socket.getOutputStream().apply {
                            write("HTTP/1.1 $code X\r\nContent-Type: application/json\r\nContent-Length: ${bytes.size}\r\nConnection: close\r\n\r\n".toByteArray())
                            write(bytes)
                            flush()
                        }
                    }
                } catch (e: java.io.IOException) {
                    // The test finished and closed the socket.
                }
            }
        }.apply { isDaemon = true }.start()
    }

    @After fun stop() = server.close()

    private fun client(token: () -> String = { "t-one" }, signedOut: () -> Unit = {}) = HqClient(token, "http://127.0.0.1:${server.localPort}", signedOut)

    private fun page(from: Int, count: Int, more: Boolean) =
        (from until from + count).joinToString(",", """{"tasks":[""", """],"has_more":$more}""") { """{"id":"id$it","display_id":"T-$it","title":"Task $it","status":"to_do","updated_at":"2026-10-21 08:00:00"}""" }

    @Test fun theTaskListPagesOnOffsetUntilTheServerHasNoMore() = runBlocking {
        answer = { q -> if (q.contains("offset=0")) 200 to page(0, 500, true) else 200 to page(500, 3, false) }
        val tasks = client().tasks()
        assertEquals(503, tasks.size)
        assertEquals("id502", tasks.last().id)
        assertEquals(2, requests.size)
        assertTrue(requests[1].contains("offset=500"))
    }

    @Test fun anAnswerOf401MeansSignedOutAndTheTokenIsReadAgainNextTime() = runBlocking {
        var reads = 0
        var signedOut = 0
        val c = client(token = { reads++; "t-$reads" }, signedOut = { signedOut++ })
        answer = { 401 to """{"error":"unauthorized"}""" }
        try { c.tasks(); fail("a 401 must not look like a list") } catch (e: HqUnavailable) { assertEquals(401, e.status) }
        assertEquals(1, signedOut)
        answer = { 200 to """{"tasks":[],"has_more":false}""" }
        c.tasks()
        c.tasks()
        assertEquals("the token is read again after a 401 and then kept", 2, reads)
    }

    @Test fun aKeystoreFaultIsAnUnavailableHqNotACrash() = runBlocking {
        val c = client(token = { throw IllegalStateException("web token unavailable") })
        try { c.tasks(); fail("no token, no request") } catch (e: HqUnavailable) { assertEquals(0, requests.size) }
    }

    @Test fun theStatusOfAnErrorAnswerIsKeptForTheScreensToWord() = runBlocking {
        answer = { 409 to """{"error":"changed"}""" }
        try { client().patchTask("x", org.json.JSONObject().put("status", "complete")); fail("a 409 is an error") } catch (e: HqUnavailable) { assertEquals(409, e.status) }
    }
}

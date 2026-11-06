package example.hq.spike.data

import java.net.InetAddress
import java.net.ServerSocket
import java.util.concurrent.CopyOnWriteArrayList
import kotlinx.coroutines.runBlocking
import org.json.JSONObject
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Before
import org.junit.Test

/** The two note calls whose bodies matter: the repeat guard on a create and the changed-elsewhere answer to a save. */
class NoteSaveTest {
    private lateinit var server: ServerSocket
    private val bodies = CopyOnWriteArrayList<String>()
    private var answer: Pair<Int, String> = 200 to "{}"

    @Before fun start() {
        server = ServerSocket(0, 50, InetAddress.getByName("127.0.0.1"))
        Thread {
            while (!server.isClosed) {
                try {
                    server.accept().use { socket ->
                        val input = socket.getInputStream().bufferedReader()
                        input.readLine() ?: return@use
                        var length = 0
                        while (true) {
                            val header = input.readLine().orEmpty()
                            if (header.isEmpty()) break
                            if (header.startsWith("Content-Length:", ignoreCase = true)) length = header.substringAfter(':').trim().toInt()
                        }
                        val buf = CharArray(length)
                        var read = 0
                        while (read < length) { val n = input.read(buf, read, length - read); if (n < 0) break; read += n }
                        bodies += String(buf, 0, read)
                        val bytes = answer.second.toByteArray()
                        socket.getOutputStream().apply {
                            write("HTTP/1.1 ${answer.first} X\r\nContent-Type: application/json\r\nContent-Length: ${bytes.size}\r\nConnection: close\r\n\r\n".toByteArray())
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

    private fun client() = HqClient({ "t" }, "http://127.0.0.1:${server.localPort}", {})

    @Test fun aNoteLinkByIdFindsWhereTheNoteLivesNow() = runBlocking {
        answer = 200 to """{"path":"Notebooks/Archive/Plan.md"}"""
        assertEquals("Notebooks/Archive/Plan.md", client().notePathById("0123456789abcdef"))
        answer = 404 to """{"error":"not found"}"""
        try {
            client().notePathById("ffffffffffffffff")
            fail("an unknown id is a missing note")
        } catch (e: NoteNotFound) {
            // The Notes gone page, not an outage.
        }
    }

    @Test fun aCreateCarriesItsClientIdOnEveryTry() = runBlocking {
        answer = 200 to """{"path":"Notebooks/Inbox/A.md"}"""
        client().createNote("Notebooks/Inbox", "A", "text", clientId = "id-1")
        client().createNote("Notebooks/Inbox", "A", "text", clientId = "id-1")
        assertEquals(listOf("id-1", "id-1"), bodies.map { JSONObject(it).getString("client_id") })
    }

    @Test fun aCreateWithNoIdSendsNone() = runBlocking {
        answer = 200 to """{"path":"Notebooks/Inbox/A.md"}"""
        client().createNote("Notebooks/Inbox", "A", "text")
        assertTrue(!JSONObject(bodies[0]).has("client_id"))
    }

    @Test fun aSaveSendsTheBaseItWasOpenedWith() = runBlocking {
        client().saveNote("Notebooks/Inbox/A.md", "new", base = "h1")
        assertEquals("h1", JSONObject(bodies[0]).getString("base"))
    }

    @Test fun aSaveToANoteThatChangedIsReportedWithWhoAndWhat() = runBlocking {
        answer = 409 to """{"changed":true,"by":"Alex","at":"2026-11-03T10:00:00Z","theirs_hash":"h2"}"""
        try {
            client().saveNote("Notebooks/Inbox/A.md", "new", base = "h1")
            fail("expected NoteChanged")
        } catch (e: NoteChanged) {
            assertEquals("h2", e.theirsHash)
            assertEquals("Alex", e.by)
        }
    }

    @Test fun aReadCarriesTheHash() = runBlocking {
        answer = 200 to """{"path":"Notebooks/Inbox/A.md","content":"x","hash":"h9"}"""
        assertEquals("h9", client().note("Notebooks/Inbox/A.md").hash)
    }
}

package example.hq.spike.data

import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import example.hq.spike.ui.chat.blocks.Attachment
import org.json.JSONArray
import org.json.JSONObject

/**
 * One web socket to the local hq that stays up: a fresh single-use ticket for every connection and a
 * growing wait between attempts. Every chat's events arrive here, so readers filter by thread.
 */
class ChatConnection(private val client: HqClient, scope: CoroutineScope) {
    private val _events = MutableSharedFlow<ChatEvent>(extraBufferCapacity = EVENT_BUFFER)
    val events: SharedFlow<ChatEvent> = _events
    private val _connected = MutableStateFlow(false)
    val connected: StateFlow<Boolean> = _connected

    @Volatile private var socket: WebSocket? = null

    init {
        scope.launch { keepConnected() }
    }

    /** False when there is no open socket, so the caller can keep the message as a draft. */
    fun sendChat(content: String, threadId: String?, clientId: String, replaceFrom: String? = null, files: List<Attachment> = emptyList()): Boolean = write(
        JSONObject().put("type", "chat").put("content", content).put("thread_id", threadId ?: JSONObject.NULL).put("client_id", clientId)
            .put("attachments", JSONArray(files.map { JSONObject().put("name", it.name).put("path", it.path).put("mime", it.mime).put("size", it.size) }))
            .apply { replaceFrom?.let { put("replace_from", it) } },
    )

    fun stop(threadId: String): Boolean = write(JSONObject().put("type", "stop").put("thread_id", threadId))

    private fun write(frame: JSONObject): Boolean = socket?.send(frame.toString()) ?: false

    /** Set when the current attempt got as far as an open socket, so a later drop retries quickly instead of at the longest wait. */
    @Volatile private var opened = false
    @Volatile private var openedAt = 0L

    private suspend fun keepConnected() {
        var wait = MIN_WAIT_MS
        while (true) {
            val closed = CompletableDeferred<Boolean>()
            opened = false
            try {
                val ticket = client.wsTicket()
                socket = client.openSocket(ticket, listener(closed))
                closed.await()
                if (opened && System.currentTimeMillis() - openedAt >= STABLE_OPEN_MS) wait = MIN_WAIT_MS
            } catch (e: HqUnavailable) {
                // hq is down or still starting; the wait below is the retry.
            } catch (e: IllegalStateException) {
                // The sealed token could not be opened yet.
            }
            socket = null
            _connected.value = false
            delay(wait)
            wait = minOf(wait * 2, MAX_WAIT_MS)
        }
    }

    private fun listener(closed: CompletableDeferred<Boolean>) = object : WebSocketListener() {
        override fun onOpen(webSocket: WebSocket, response: Response) {
            opened = true
            openedAt = System.currentTimeMillis()
            _connected.value = true
        }

        override fun onMessage(webSocket: WebSocket, text: String) {
            ChatEvent.parse(text)?.let { _events.tryEmit(it) }
        }

        override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
            closed.complete(true)
        }

        override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
            closed.complete(false)
        }
    }

    private companion object {
        const val EVENT_BUFFER = 512
        const val MIN_WAIT_MS = 1_000L
        const val MAX_WAIT_MS = 15_000L
        /** A connection that lasted this long was a working one; a shorter one keeps the growing wait. */
        const val STABLE_OPEN_MS = 5_000L
    }
}

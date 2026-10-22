package example.hq.spike.bridge

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Each line is a line `hq bridge run` really printed (see crates/hq-cli/src/commands/bridge_control.rs). */
class BridgeWireTest {
    private fun parse(line: String) = BridgeWire.parse(line)

    @Test fun readyNamesTheDeviceAndTheRelay() {
        val e = parse("""{"event":"ready","id":"66902dde","name":"emu","relay":"default","version":1}""") as BridgeEvent.Ready
        assertEquals("66902dde", e.id)
        assertEquals("emu", e.name)
        assertEquals("default", e.relay)
    }

    @Test fun aTicketCarriesTheLinkAndItsLifetime() {
        val e = parse("""{"event":"ticket","expires_in_secs":60,"link":"hqpair://ticket/abc"}""") as BridgeEvent.Ticket
        assertEquals("hqpair://ticket/abc", e.link)
        assertEquals(60, e.expiresInSecs)
    }

    @Test fun aRequestNeverCarriesTheDigits() {
        val e = parse("""{"event":"request","joiner":"ab12","name":"Pixel","window_secs":120}""") as BridgeEvent.Request
        assertEquals(BridgeEvent.Request("ab12", "Pixel", 120), e)
    }

    @Test fun peersKeepTheirTimesAndRevocations() {
        val e = parse("""{"event":"peers","peers":[{"id":"aa","name":"Desk","paired_at":100,"last_seen_at":null,"revoked_at":null},{"id":"bb","name":"Old","paired_at":50,"last_seen_at":60,"revoked_at":70}]}""") as BridgeEvent.Peers
        assertEquals(2, e.peers.size)
        assertEquals(BridgePeer("aa", "Desk", 100, null, null), e.peers[0])
        assertTrue(e.peers[0].active)
        assertFalse(e.peers[1].active)
        assertEquals(60L, e.peers[1].lastSeenAt)
    }

    @Test fun aPingReportsAnErrorOrTimes() {
        val ok = parse("""{"event":"ping","id":"aa","connect_ms":900,"median_ms":6,"path":"direct","error":null}""") as BridgeEvent.Ping
        assertEquals(BridgeEvent.Ping("aa", 900, 6, "direct", null), ok)
        val bad = parse("""{"event":"ping","id":"aa","error":"that device is not paired"}""") as BridgeEvent.Ping
        assertEquals("that device is not paired", bad.error)
        assertNull(bad.medianMs)
    }

    @Test fun anUnknownEventOrABrokenLineIsSkippedNotFatal() {
        assertNull(parse("""{"event":"from_the_future"}"""))
        assertNull(parse("not json"))
        assertNull(parse(""))
    }

    @Test fun errorsKeepTheCommandThatFailed() {
        assertEquals(BridgeEvent.Failed("digits", "no device is waiting for digits"), parse("""{"event":"error","cmd":"digits","message":"no device is waiting for digits"}"""))
    }

    @Test fun commandsAreOneLineOfJson() {
        val line = BridgeWire.open(60)
        assertFalse(line.contains('\n'))
        assertEquals("open", JSONObject(line).getString("cmd"))
        assertEquals(60, JSONObject(line).getInt("ttl_secs"))
        assertEquals("123456", JSONObject(BridgeWire.digits("123456")).getString("value"))
        assertEquals("network_change", JSONObject(BridgeWire.networkChange()).getString("cmd"))
        assertEquals("quit", JSONObject(BridgeWire.quit()).getString("cmd"))
    }

    @Test fun aLinkWithQuotesStaysOneLine() {
        val line = BridgeWire.join("hqpair://ticket/a\"b\\c")
        assertFalse(line.contains('\n'))
        assertEquals("hqpair://ticket/a\"b\\c", JSONObject(line).getString("link"))
    }

    @Test fun theRelayFileCanOnlyHoldWhatHqTakes() {
        assertEquals("none", BridgeSupervisor.relayOrDefault("none"))
        assertEquals("default", BridgeSupervisor.relayOrDefault(null))
        assertEquals("https://relay.example.com", BridgeSupervisor.relayOrDefault("https://relay.example.com"))
        assertEquals("default", BridgeSupervisor.relayOrDefault("--no-addrs"))
        assertEquals("default", BridgeSupervisor.relayOrDefault("https://relay.example.com --bind 0.0.0.0:1"))
    }
}

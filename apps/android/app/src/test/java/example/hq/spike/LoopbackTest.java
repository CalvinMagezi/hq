package example.hq.spike;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertNull;

import java.net.Proxy;
import java.net.URL;
import org.junit.Test;

/** A carrier APN proxy cannot reach the phone's own hq, so loopback calls must go direct. */
public class LoopbackTest {
    @Test
    public void loopbackUrlsNeverUseAProxy() throws Exception {
        for (String u : new String[] {"http://127.0.0.1:5678/health", "http://localhost:5678/", "http://[::1]:5678/"}) {
            assertEquals(u, Proxy.NO_PROXY, Loopback.proxyFor(new URL(u)));
        }
    }

    @Test
    public void otherHostsKeepTheSystemsChoice() throws Exception {
        assertNull(Loopback.proxyFor(new URL("https://hq.example.ts.net/health")));
        assertNull(Loopback.proxyFor(new URL("http://127.example.com/")));
    }
}

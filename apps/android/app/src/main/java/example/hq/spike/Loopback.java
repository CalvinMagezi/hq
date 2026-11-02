package example.hq.spike;

import java.io.IOException;
import java.net.HttpURLConnection;
import java.net.Proxy;
import java.net.URL;

/**
 * Connections to the hq on this phone. A mobile network can carry an HTTP proxy in its APN, which
 * Android applies to every request, loopback included; the proxy cannot reach this phone's own
 * 127.0.0.1, so a call to hq through it never arrives. Loopback addresses never use a proxy.
 */
public final class Loopback {
    private static final java.util.regex.Pattern IPV4_LOOPBACK =
            java.util.regex.Pattern.compile("127(\\.\\d{1,3}){3}");

    private Loopback() {}

    /** Whether host names this phone itself. */
    public static boolean isLoopback(String host) {
        if (host == null) return false;
        String h = host.toLowerCase(java.util.Locale.ROOT);
        return h.equals("localhost") || h.equals("::1") || h.equals("[::1]") || IPV4_LOOPBACK.matcher(h).matches();
    }

    /** No proxy for a loopback URL; the system's choice for any other. */
    public static Proxy proxyFor(URL url) {
        return isLoopback(url.getHost()) ? Proxy.NO_PROXY : null;
    }

    /** Opens url, directly when it names this phone. */
    public static HttpURLConnection open(URL url) throws IOException {
        Proxy proxy = proxyFor(url);
        return (HttpURLConnection) (proxy == null ? url.openConnection() : url.openConnection(proxy));
    }
}

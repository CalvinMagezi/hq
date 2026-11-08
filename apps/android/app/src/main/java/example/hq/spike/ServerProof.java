package example.hq.spike;

import java.io.IOException;
import java.io.InputStream;
import java.net.HttpURLConnection;
import java.net.URL;
import java.nio.charset.StandardCharsets;
import java.security.GeneralSecurityException;
import java.security.MessageDigest;
import java.security.SecureRandom;
import java.util.Base64;
import javax.crypto.Mac;
import javax.crypto.spec.SecretKeySpec;
import org.json.JSONException;
import org.json.JSONObject;

/**
 * Checks that the server on this phone's HQ port is the hq that holds this app's web token, without sending the
 * token. Any installed app can listen on 127.0.0.1 while hq is not holding the port (at boot, while it restarts,
 * after a crash, during a restore), so nothing is sent with the token until the server has answered a fresh nonce
 * with HMAC-SHA256(token, "hq-server-proof/1" || nonce) (docs/security/WEB_AUTH.md, "Proving the server is HQ").
 * A proof holds for the hq child it was made against; a new child, or a request that could not connect, needs a
 * new one. Every call here may use the network: never call it on the main thread.
 */
public final class ServerProof {
    private static final String LABEL = "hq-server-proof/1";
    private static final String HMAC = "HmacSHA256";
    private static final int NONCE_BYTES = 32;
    private static final int TIMEOUT_MS = 3_000;
    private static final int MAX_BODY_BYTES = 64 * 1024;
    private static final SecureRandom RANDOM = new SecureRandom();

    private static final Object LOCK = new Object();
    /** The hq child that proved itself last, and a digest of the token it proved; both null when none has. */
    private static Process provenChild;
    private static byte[] provenToken;

    private ServerProof() {}

    /** The phone's own hq, the one server whose port another app could take. */
    public static String localBase() {
        return "http://127.0.0.1:" + HubService.PORT;
    }

    /** Whether base names the phone's own hq. Any other server is reached with its own token, and is left as it is. */
    static boolean checks(String base) {
        try {
            URL u = new URL(base);
            return Loopback.isLoopback(u.getHost()) && u.getPort() == HubService.PORT;
        } catch (IOException e) {
            return false;
        }
    }

    /**
     * True when it is safe to send token to base: base is not the phone's own hq, or that hq proved it holds token,
     * now or earlier while the same hq child still runs. False means: send nothing with the token.
     */
    public static boolean ensure(String base, String token) {
        if (!checks(base)) return true;
        if (token == null || token.isEmpty()) return false;
        Process child = HubService.hqChild();
        byte[] digest = sha256(token);
        synchronized (LOCK) {
            if (child != null && child == provenChild && child.isAlive() && MessageDigest.isEqual(digest, provenToken)) return true;
        }
        if (!prove(base, token)) return false;
        synchronized (LOCK) {
            // Kept only for the child that was running before and after the answer; another one asks again.
            if (child != null && child == HubService.hqChild()) {
                provenChild = child;
                provenToken = digest;
            }
        }
        return true;
    }

    /** Asks again, whatever an earlier proof said: before acting on a 401, which a stranger on the port could send. */
    public static boolean proveNow(String base, String token) {
        forget();
        return ensure(base, token);
    }

    /** Drops the proof, so the next request with the token asks first: a request to hq could not connect. */
    public static void forget() {
        synchronized (LOCK) {
            provenChild = null;
            provenToken = null;
        }
    }

    /** One round: a fresh nonce to base's /api/hello, and whether the answer is the one only token's holder can give. */
    static boolean prove(String base, String token) {
        byte[] nonce = new byte[NONCE_BYTES];
        RANDOM.nextBytes(nonce);
        String encoded = Base64.getUrlEncoder().withoutPadding().encodeToString(nonce);
        HttpURLConnection h = null;
        try {
            h = Loopback.open(new URL(base + "/api/hello?proof=" + encoded));
            h.setConnectTimeout(TIMEOUT_MS);
            h.setReadTimeout(TIMEOUT_MS);
            h.setInstanceFollowRedirects(false);
            h.setUseCaches(false);
            if (h.getResponseCode() != HttpURLConnection.HTTP_OK) return false;
            String proof = new JSONObject(read(h.getInputStream())).optString("proof", "");
            return matches(token, nonce, proof);
        } catch (IOException | JSONException | RuntimeException e) {
            return false;
        } finally {
            if (h != null) h.disconnect();
        }
    }

    /** Whether proof (base64url) is HMAC-SHA256(token, LABEL || nonce). */
    static boolean matches(String token, byte[] nonce, String proof) {
        if (proof == null || proof.isEmpty()) return false;
        byte[] got;
        try {
            got = Base64.getUrlDecoder().decode(proof);
        } catch (IllegalArgumentException e) {
            return false;
        }
        return MessageDigest.isEqual(answer(token, nonce), got);
    }

    static byte[] answer(String token, byte[] nonce) {
        try {
            Mac mac = Mac.getInstance(HMAC);
            mac.init(new SecretKeySpec(token.getBytes(StandardCharsets.UTF_8), HMAC));
            mac.update(LABEL.getBytes(StandardCharsets.US_ASCII));
            return mac.doFinal(nonce);
        } catch (GeneralSecurityException e) {
            throw new IllegalStateException(e);
        }
    }

    private static byte[] sha256(String token) {
        try {
            return MessageDigest.getInstance("SHA-256").digest(token.getBytes(StandardCharsets.UTF_8));
        } catch (GeneralSecurityException e) {
            throw new IllegalStateException(e);
        }
    }

    private static String read(InputStream in) throws IOException {
        try (InputStream s = in) {
            byte[] buf = new byte[MAX_BODY_BYTES];
            int n = 0;
            for (int r; n < buf.length && (r = s.read(buf, n, buf.length - n)) > 0; ) n += r;
            return new String(buf, 0, n, StandardCharsets.UTF_8);
        }
    }
}

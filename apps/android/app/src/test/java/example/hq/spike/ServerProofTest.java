package example.hq.spike;

import static org.junit.Assert.assertArrayEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;

import java.util.Base64;
import org.junit.Test;

/** The app sends its web token only to a server that answered the nonce the way only HQ can (docs/security/WEB_AUTH.md). */
public class ServerProofTest {
    private static final String TOKEN = "example-web-token";
    /** The vector in crates/hq-core/src/server_proof.rs: nonce bytes 0..31. */
    private static final String PROOF = "s887Mn5Ba7kWekr_rkJlZl5zEYzF3_5f3jGwuiy_PXk";

    private static byte[] nonce() {
        byte[] n = new byte[32];
        for (int i = 0; i < n.length; i++) n[i] = (byte) i;
        return n;
    }

    @Test
    public void theAnswerMatchesHqsVector() {
        assertArrayEquals(Base64.getUrlDecoder().decode(PROOF), ServerProof.answer(TOKEN, nonce()));
        assertTrue(ServerProof.matches(TOKEN, nonce(), PROOF));
    }

    @Test
    public void anotherTokenNonceOrAnswerIsRefused() {
        assertFalse(ServerProof.matches("another-token", nonce(), PROOF));
        byte[] other = nonce();
        other[0] = 1;
        assertFalse(ServerProof.matches(TOKEN, other, PROOF));
        assertFalse(ServerProof.matches(TOKEN, nonce(), ""));
        assertFalse(ServerProof.matches(TOKEN, nonce(), null));
        assertFalse(ServerProof.matches(TOKEN, nonce(), "not base64!"));
        assertFalse(ServerProof.matches(TOKEN, nonce(), PROOF.substring(1)));
    }

    @Test
    public void onlyThePhonesOwnHqIsChecked() {
        assertTrue(ServerProof.checks(ServerProof.localBase()));
        assertTrue(ServerProof.checks("http://localhost:" + HubService.PORT));
        assertFalse(ServerProof.checks("https://hq.example.ts.net"));
        assertFalse(ServerProof.checks("http://127.0.0.1:" + (HubService.PORT + 1)));
    }
}

package example.hq.spike;

import android.content.Context;
import android.content.SharedPreferences;
import android.security.keystore.KeyGenParameterSpec;
import android.security.keystore.KeyProperties;
import android.util.Base64;
import android.util.Log;
import java.io.File;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.LinkOption;
import java.nio.file.Files;
import java.security.GeneralSecurityException;
import java.security.KeyStore;
import java.security.SecureRandom;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import javax.crypto.AEADBadTagException;
import javax.crypto.Cipher;
import javax.crypto.KeyGenerator;
import javax.crypto.SecretKey;
import javax.crypto.spec.GCMParameterSpec;

/**
 * Credentials for the hq child, kept as AES-GCM ciphertext under a key that lives in the Android
 * Keystore. The owner drops a plain NAME=value file in the app's private folder; it is encrypted
 * on the next child start and wiped. The name is bound to its ciphertext as associated data.
 */
final class SecretStore {
    private static final String KEYSTORE = "AndroidKeyStore";
    private static final String ALIAS = "hq-secrets";
    private static final String PREFS = "secrets";
    private static final String INTERNAL_PREFS = "internal";
    private static final String WEB_TOKEN = "web_token";
    private static final String SERVER_TOKEN = "server_token";
    private static final int TOKEN_BYTES = 18;
    private static final String TRANSFORM = "AES/GCM/NoPadding";
    private static final int KEY_BITS = 256;
    private static final int TAG_BITS = 128;
    private static final String SEP = ":";

    private SecretStore() { }

    /** Applies a pending secrets file, if there is one, and returns what changed as +NAME or -NAME. */
    static List<String> importPending(Context c, File inbox) {
        List<String> changed = new ArrayList<>();
        if (!Files.exists(inbox.toPath(), LinkOption.NOFOLLOW_LINKS)) return changed;
        try {
            String text = new String(HubConfig.readRegular(inbox.toPath(), HubConfig.MAX_FILE_BYTES), StandardCharsets.UTF_8);
            Map<String, String> entries = HubConfig.parseSecrets(text);
            SharedPreferences.Editor edit = c.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit();
            for (Map.Entry<String, String> e : entries.entrySet()) {
                if (e.getValue().isEmpty()) {
                    edit.remove(e.getKey());
                    changed.add("-" + e.getKey());
                } else {
                    edit.putString(e.getKey(), seal(e.getKey(), e.getValue()));
                    changed.add("+" + e.getKey());
                }
            }
            if (!edit.commit()) throw new IOException("preferences not written; values may be held in memory until the app restarts");
            Log.i(HubService.TAG, "secrets imported: " + changed);
        } catch (Exception e) {
            changed.clear();
            Log.e(HubService.TAG, "secrets file refused: " + e.getMessage());
        } finally {
            try {
                HubConfig.wipe(inbox.toPath());
            } catch (IOException e) {
                Log.e(HubService.TAG, "could not remove the secrets file: " + e.getMessage());
            }
        }
        return changed;
    }

    /**
     * Seals one credential, under the same rules as a line of secrets.in. The value goes from memory to the
     * encrypted preferences and nowhere else. Throws IllegalArgumentException (the reason, never the value)
     * for a name or value the rules refuse, and IllegalStateException when it could not be stored.
     */
    static synchronized void put(Context c, String name, String value) {
        String problem = value.isEmpty() ? "value is empty" : HubConfig.entryProblem(name, value);
        if (problem != null) throw new IllegalArgumentException(problem);
        SharedPreferences prefs = c.getSharedPreferences(PREFS, Context.MODE_PRIVATE);
        if (!prefs.contains(name) && prefs.getAll().size() >= HubConfig.MAX_SECRETS) throw new IllegalArgumentException("too many names");
        try {
            if (!prefs.edit().putString(name, seal(name, value)).commit()) throw new IOException("preferences not written");
        } catch (GeneralSecurityException | IOException e) {
            throw new IllegalStateException("key not stored: " + e.getClass().getSimpleName());
        }
    }

    /** Forgets one stored credential; a name that is not stored is not an error. */
    static synchronized void remove(Context c, String name) {
        if (!c.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().remove(name).commit()) {
            throw new IllegalStateException("key not removed");
        }
    }

    /** The stored names a credential may have, sorted; never a value. */
    static synchronized List<String> names(Context c) {
        List<String> out = new ArrayList<>();
        for (String name : c.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getAll().keySet()) {
            if (HubConfig.isCredentialName(name)) out.add(name);
        }
        java.util.Collections.sort(out);
        return out;
    }

    /**
     * The token that gates the web app and API. It is kept sealed like the credentials; one left in
     * plain preferences by an earlier build is moved here on first use. A new token is made only when
     * the stored one is unreadable for good (the key was replaced or the value is damaged); any other
     * failure is thrown so the caller retries, because a new token locks out every signed-in device.
     */
    static synchronized String webToken(Context c) {
        SharedPreferences internal = c.getSharedPreferences(INTERNAL_PREFS, Context.MODE_PRIVATE);
        SharedPreferences legacy = c.getSharedPreferences("hq", Context.MODE_PRIVATE);
        try {
            String stored = internal.getString(WEB_TOKEN, null);
            if (stored != null) {
                try {
                    String token = open(WEB_TOKEN, stored);
                    if (legacy.contains("token")) legacy.edit().remove("token").apply();
                    return token;
                } catch (AEADBadTagException | IllegalArgumentException | ArrayIndexOutOfBoundsException lost) {
                    Log.e(HubService.TAG, "web token is unreadable for good, making a new one: " + lost.getClass().getSimpleName());
                }
            }
            String token = legacy.getString("token", null);
            if (token == null) token = newToken();
            if (!internal.edit().putString(WEB_TOKEN, seal(WEB_TOKEN, token)).commit()) throw new IOException("preferences not written");
            legacy.edit().remove("token").apply();
            return token;
        } catch (GeneralSecurityException | IOException e) {
            throw new IllegalStateException("web token unavailable: " + e.getClass().getSimpleName());
        }
    }

    /** Seals a fresh web token. hq must be restarted to read it; every device signed in with the old one keeps its own session. */
    static synchronized void replaceWebToken(Context c) {
        SharedPreferences internal = c.getSharedPreferences(INTERNAL_PREFS, Context.MODE_PRIVATE);
        try {
            if (!internal.edit().putString(WEB_TOKEN, seal(WEB_TOKEN, newToken())).commit()) throw new IOException("preferences not written");
        } catch (GeneralSecurityException | IOException e) {
            throw new IllegalStateException("web token not replaced: " + e.getClass().getSimpleName());
        }
    }

    /**
     * The device token this phone holds for the owner's HQ on a server (ServerHq), sealed like the web
     * token; null when none is kept or it can no longer be opened, which means signing in again.
     */
    static synchronized String serverToken(Context c) {
        String stored = c.getSharedPreferences(INTERNAL_PREFS, Context.MODE_PRIVATE).getString(SERVER_TOKEN, null);
        if (stored == null) return null;
        try {
            return open(SERVER_TOKEN, stored);
        } catch (Exception e) {
            Log.e(HubService.TAG, "server token cannot be opened: " + e.getClass().getSimpleName());
            return null;
        }
    }

    static synchronized void keepServerToken(Context c, String token) {
        SharedPreferences internal = c.getSharedPreferences(INTERNAL_PREFS, Context.MODE_PRIVATE);
        try {
            if (!internal.edit().putString(SERVER_TOKEN, seal(SERVER_TOKEN, token)).commit()) throw new IOException("preferences not written");
        } catch (GeneralSecurityException | IOException e) {
            throw new IllegalStateException("server token not kept: " + e.getClass().getSimpleName());
        }
    }

    static synchronized void forgetServerToken(Context c) {
        c.getSharedPreferences(INTERNAL_PREFS, Context.MODE_PRIVATE).edit().remove(SERVER_TOKEN).commit();
    }

    private static String newToken() {
        byte[] b = new byte[TOKEN_BYTES];
        new SecureRandom().nextBytes(b);
        StringBuilder s = new StringBuilder("t");
        for (byte x : b) s.append(String.format("%02x", x));
        return s.toString();
    }

    /**
     * Every stored secret, decrypted. A name the current rule no longer allows (kept by an older build)
     * is dropped from the store and never reaches the child. A value that cannot be opened is logged
     * by name and skipped.
     */
    static Map<String, String> load(Context c) {
        SharedPreferences prefs = c.getSharedPreferences(PREFS, Context.MODE_PRIVATE);
        Map<String, String> out = new LinkedHashMap<>();
        SharedPreferences.Editor drop = prefs.edit();
        boolean dropped = false;
        for (Map.Entry<String, ?> e : prefs.getAll().entrySet()) {
            if (!HubConfig.isCredentialName(e.getKey())) {
                drop.remove(e.getKey());
                dropped = true;
                Log.w(HubService.TAG, "stored name " + e.getKey() + " is not allowed any more, removed");
                continue;
            }
            try {
                out.put(e.getKey(), open(e.getKey(), (String) e.getValue()));
            } catch (Exception ex) {
                Log.e(HubService.TAG, "secret " + e.getKey() + " cannot be opened: " + ex.getClass().getSimpleName());
            }
        }
        if (dropped) drop.commit();
        return out;
    }

    private static synchronized SecretKey key() throws GeneralSecurityException, IOException {
        KeyStore ks = KeyStore.getInstance(KEYSTORE);
        ks.load(null);
        if (ks.containsAlias(ALIAS)) return (SecretKey) ks.getKey(ALIAS, null);
        KeyGenerator gen = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, KEYSTORE);
        gen.init(new KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT | KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(KEY_BITS)
                .build());
        return gen.generateKey();
    }

    private static String seal(String name, String value) throws GeneralSecurityException, IOException {
        Cipher cipher = Cipher.getInstance(TRANSFORM);
        cipher.init(Cipher.ENCRYPT_MODE, key());
        cipher.updateAAD(name.getBytes(StandardCharsets.UTF_8));
        byte[] ct = cipher.doFinal(value.getBytes(StandardCharsets.UTF_8));
        return Base64.encodeToString(cipher.getIV(), Base64.NO_WRAP) + SEP + Base64.encodeToString(ct, Base64.NO_WRAP);
    }

    private static String open(String name, String stored) throws GeneralSecurityException, IOException {
        String[] parts = stored.split(SEP, 2);
        Cipher cipher = Cipher.getInstance(TRANSFORM);
        cipher.init(Cipher.DECRYPT_MODE, key(), new GCMParameterSpec(TAG_BITS, Base64.decode(parts[0], Base64.NO_WRAP)));
        cipher.updateAAD(name.getBytes(StandardCharsets.UTF_8));
        return new String(cipher.doFinal(Base64.decode(parts[1], Base64.NO_WRAP)), StandardCharsets.UTF_8);
    }
}

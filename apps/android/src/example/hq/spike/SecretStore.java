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
     * The token that gates the web app and API. It is kept sealed like the credentials; one left in
     * plain preferences by an earlier build is moved here on first use.
     */
    static synchronized String webToken(Context c) {
        SharedPreferences internal = c.getSharedPreferences(INTERNAL_PREFS, Context.MODE_PRIVATE);
        try {
            String stored = internal.getString(WEB_TOKEN, null);
            if (stored != null) return open(WEB_TOKEN, stored);
        } catch (GeneralSecurityException | IOException e) {
            Log.e(HubService.TAG, "web token cannot be opened, making a new one: " + e.getClass().getSimpleName());
        }
        SharedPreferences legacy = c.getSharedPreferences("hq", Context.MODE_PRIVATE);
        String token = legacy.getString("token", null);
        if (token == null) token = newToken();
        try {
            if (!internal.edit().putString(WEB_TOKEN, seal(WEB_TOKEN, token)).commit()) throw new IOException("preferences not written");
            legacy.edit().remove("token").commit();
        } catch (GeneralSecurityException | IOException e) {
            throw new IllegalStateException("web token cannot be stored: " + e.getMessage());
        }
        return token;
    }

    private static String newToken() {
        byte[] b = new byte[TOKEN_BYTES];
        new SecureRandom().nextBytes(b);
        StringBuilder s = new StringBuilder("t");
        for (byte x : b) s.append(String.format("%02x", x));
        return s.toString();
    }

    /** Every stored secret, decrypted. A value that cannot be opened is logged by name and skipped. */
    static Map<String, String> load(Context c) {
        Map<String, String> out = new LinkedHashMap<>();
        for (Map.Entry<String, ?> e : c.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getAll().entrySet()) {
            try {
                out.put(e.getKey(), open(e.getKey(), (String) e.getValue()));
            } catch (Exception ex) {
                Log.e(HubService.TAG, "secret " + e.getKey() + " cannot be opened: " + ex.getClass().getSimpleName());
            }
        }
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

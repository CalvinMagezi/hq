package example.hq.spike;

import java.io.IOException;
import java.io.RandomAccessFile;
import java.nio.file.Files;
import java.nio.file.LinkOption;
import java.nio.file.NoSuchFileException;
import java.nio.file.Path;
import java.nio.file.attribute.BasicFileAttributes;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.regex.Pattern;

/** Parsing, file handling and decisions that need no Android classes, so apps/android/check.sh can test them on a plain JDK. */
final class HubConfig {
    static final int MAX_SECRETS = 64;
    static final int MAX_VALUE_CHARS = 4096;
    static final long MAX_FILE_BYTES = 64 * 1024;
    static final String MODE_ACTIVE = "active";

    // Credentials only. A name that ends in _API_KEY or _TOKEN cannot be one of the switches hq reads
    // straight from the environment (HQ_MCP_DEV_NO_AUTH, HQ_BIN_PATH, HQ_VAULT_PATH, ...), and LD_PRELOAD,
    // PATH and HOME never match.
    private static final Pattern NAME = Pattern.compile("[A-Z][A-Z0-9_]{0,55}_(API_KEY|TOKEN)");
    // Names that match the rule above but that the app sets itself.
    private static final String RESERVED = "HQ_WEB_AUTH_TOKEN";
    private static final int ZERO_CHUNK = 4096;

    private HubConfig() { }

    /**
     * Reads the lines of a secrets file: NAME=value, '#' comments and blank lines ignored. An empty
     * value asks for the name to be removed. Errors name the line, never the value.
     */
    static Map<String, String> parseSecrets(String text) {
        Map<String, String> out = new LinkedHashMap<>();
        String[] lines = text.split("\n", -1);
        for (int i = 0; i < lines.length; i++) {
            String line = lines[i].endsWith("\r") ? lines[i].substring(0, lines[i].length() - 1) : lines[i];
            if (line.isEmpty() || line.startsWith("#")) continue;
            int eq = line.indexOf('=');
            if (eq < 1) throw bad(i, "expected NAME=value");
            String name = line.substring(0, eq);
            String value = line.substring(eq + 1);
            if (!NAME.matcher(name).matches()) throw bad(i, "name not allowed");
            if (RESERVED.equals(name)) throw bad(i, "name reserved for the app");
            if (value.length() > MAX_VALUE_CHARS) throw bad(i, "value too long");
            if (hasControlChar(value)) throw bad(i, "control character in value");
            if (out.put(name, value) != null) throw bad(i, "name repeated");
            if (out.size() > MAX_SECRETS) throw bad(i, "too many names");
        }
        return out;
    }

    /** The child's environment: credentials first, then the app's own settings, which always win. */
    static Map<String, String> childEnv(Map<String, String> secrets, Map<String, String> fixed) {
        Map<String, String> env = new LinkedHashMap<>(secrets);
        env.putAll(fixed);
        return env;
    }

    /** The relays and the scheduler run only when files/mode says "active"; anything else is standby. */
    static boolean isActive(String modeFile) {
        return modeFile != null && modeFile.trim().equals(MODE_ACTIVE);
    }

    /** Standby serves the web app only; active is the daemon with relays, which serves it too. */
    static String[] hqArgs(boolean active, int port) {
        if (active) return new String[] {"start", "all"};
        return new String[] {"web", "--no-open", "--port", String.valueOf(port), "--bind", "127.0.0.1"};
    }

    /** Reads a regular file of at most max bytes; a link, a directory or a larger file is refused. */
    static byte[] readRegular(Path p, long max) throws IOException {
        BasicFileAttributes a = Files.readAttributes(p, BasicFileAttributes.class, LinkOption.NOFOLLOW_LINKS);
        if (!a.isRegularFile()) throw new IOException("not a regular file");
        if (a.size() > max) throw new IOException("file larger than " + max + " bytes");
        return Files.readAllBytes(p);
    }

    /**
     * Overwrites a small regular file with zeros and deletes it. A link is only unlinked, never
     * written through, and a file over the size cap is deleted without being rewritten. The delete
     * happens even when the overwrite fails.
     */
    static void wipe(Path p) throws IOException {
        try {
            BasicFileAttributes a = Files.readAttributes(p, BasicFileAttributes.class, LinkOption.NOFOLLOW_LINKS);
            if (a.isRegularFile() && a.size() <= MAX_FILE_BYTES) zero(p, a.size());
        } catch (NoSuchFileException gone) {
            return;
        } finally {
            Files.deleteIfExists(p);
        }
    }

    private static void zero(Path p, long length) throws IOException {
        byte[] chunk = new byte[ZERO_CHUNK];
        try (RandomAccessFile raf = new RandomAccessFile(p.toFile(), "rw")) {
            for (long done = 0; done < length; done += chunk.length) {
                raf.write(chunk, 0, (int) Math.min(chunk.length, length - done));
            }
            raf.getFD().sync();
        }
    }

    private static boolean hasControlChar(String s) {
        for (int i = 0; i < s.length(); i++) {
            char c = s.charAt(i);
            if (c < 0x20 || c == 0x7f) return true;
        }
        return false;
    }

    private static IllegalArgumentException bad(int zeroBasedLine, String why) {
        return new IllegalArgumentException("line " + (zeroBasedLine + 1) + ": " + why);
    }
}

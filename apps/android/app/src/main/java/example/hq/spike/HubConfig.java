package example.hq.spike;

import java.io.IOException;
import java.nio.ByteBuffer;
import java.nio.channels.FileChannel;
import java.nio.file.Files;
import java.nio.file.LinkOption;
import java.nio.file.NoSuchFileException;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;
import java.nio.file.attribute.BasicFileAttributes;
import java.util.ArrayList;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.regex.Pattern;

/** Parsing, file handling and decisions that need no Android classes, so apps/android/check.sh can test them on a plain JDK. */
final class HubConfig {
    static final int MAX_SECRETS = 64;
    static final int MAX_VALUE_CHARS = 4096;
    static final long MAX_FILE_BYTES = 64 * 1024;
    static final String MODE_ACTIVE = "active";
    static final int SEEN_CAP = 300;

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
            if (!isCredentialName(name)) throw bad(i, RESERVED.equals(name) ? "name reserved for the app" : "name not allowed");
            if (value.length() > MAX_VALUE_CHARS) throw bad(i, "value too long");
            if (hasControlChar(value)) throw bad(i, "control character in value");
            if (out.put(name, value) != null) throw bad(i, "name repeated");
            if (out.size() > MAX_SECRETS) throw bad(i, "too many names");
        }
        return out;
    }

    /** True for a name a credential file may set; the same rule filters what is already stored. */
    static boolean isCredentialName(String name) {
        return NAME.matcher(name).matches() && !RESERVED.equals(name);
    }

    /** Ids in the order given that are not in seen. */
    static List<String> unseen(List<String> ids, List<String> seen) {
        Set<String> known = new HashSet<>(seen);
        List<String> out = new ArrayList<>();
        for (String id : ids) {
            if (known.add(id)) out.add(id);
        }
        return out;
    }

    /** The seen list after adding ids: no repeats, newest last, the oldest dropped past the cap. */
    static List<String> remember(List<String> seen, List<String> added, int cap) {
        List<String> out = new ArrayList<>();
        Set<String> kept = new HashSet<>();
        for (List<String> part : java.util.Arrays.asList(seen, added)) {
            for (String id : part) {
                if (kept.add(id)) out.add(id);
            }
        }
        return out.size() <= cap ? out : new ArrayList<>(out.subList(out.size() - cap, out.size()));
    }

    /** A page to open from outside the app: a short lowercase path, else the home page. */
    static String safePath(String path) {
        return path != null && path.matches("/[a-z]{1,20}") ? path : "/";
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

    /** Standby serves the web app only; active is the daemon with relays, which serves it too. Supervised keeps the sign-in token out of the log. */
    static String[] hqArgs(boolean active, int port) {
        if (active) return new String[] {"start", "all"};
        return new String[] {"web", "--no-open", "--supervised", "--port", String.valueOf(port), "--bind", "127.0.0.1"};
    }

    /**
     * Reads a regular file of at most max bytes; a link, a directory or a larger file is refused.
     * The attribute check gives the reason, and the open itself refuses a link, so a swap between
     * the two cannot read through one.
     */
    static byte[] readRegular(Path p, long max) throws IOException {
        BasicFileAttributes a = Files.readAttributes(p, BasicFileAttributes.class, LinkOption.NOFOLLOW_LINKS);
        if (!a.isRegularFile()) throw new IOException("not a regular file");
        if (a.size() > max) throw new IOException("file larger than " + max + " bytes");
        try (FileChannel ch = FileChannel.open(p, StandardOpenOption.READ, LinkOption.NOFOLLOW_LINKS)) {
            if (ch.size() > max) throw new IOException("file larger than " + max + " bytes");
            ByteBuffer buf = ByteBuffer.allocate((int) ch.size());
            while (buf.hasRemaining() && ch.read(buf) >= 0) { /* keep reading */ }
            return buf.array();
        }
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
        ByteBuffer chunk = ByteBuffer.allocate(ZERO_CHUNK);
        try (FileChannel ch = FileChannel.open(p, StandardOpenOption.WRITE, LinkOption.NOFOLLOW_LINKS)) {
            for (long done = 0; done < length; done += ZERO_CHUNK) {
                chunk.clear().limit((int) Math.min(ZERO_CHUNK, length - done));
                while (chunk.hasRemaining()) ch.write(chunk, done + chunk.position());
            }
            ch.force(true);
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

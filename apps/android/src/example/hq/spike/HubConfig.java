package example.hq.spike;

import java.util.LinkedHashMap;
import java.util.Map;
import java.util.regex.Pattern;

/** Parsing and decisions that need no Android classes, so apps/android/check.sh can test them on a plain JDK. */
final class HubConfig {
    static final int MAX_SECRETS = 64;
    static final int MAX_VALUE_CHARS = 4096;
    static final String MODE_ACTIVE = "active";

    // HQ_* reaches every config key, *_API_KEY and *_TOKEN cover the names backends read credentials
    // from. LD_PRELOAD, PATH and HOME can never match.
    private static final Pattern NAME = Pattern.compile("HQ_[A-Z0-9_]{1,60}|[A-Z][A-Z0-9_]{0,55}_(API_KEY|TOKEN)");

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
            if (value.length() > MAX_VALUE_CHARS) throw bad(i, "value too long");
            if (hasControlChar(value)) throw bad(i, "control character in value");
            if (out.put(name, value) != null) throw bad(i, "name repeated");
            if (out.size() > MAX_SECRETS) throw bad(i, "too many names");
        }
        return out;
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

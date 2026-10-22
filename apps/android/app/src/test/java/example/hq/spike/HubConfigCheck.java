package example.hq.spike;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.LinkedHashMap;
import java.util.Map;

/** The checks behind HubConfigTest; assertions must be on (Gradle tests enable them). */
public class HubConfigCheck {
    private static final String MARK = "SECRETVALUE";

    public static void main(String[] args) throws Exception {
        parsing();
        rejections();
        environment();
        alerts();
        files();
        restarts();
        System.out.println("HubConfig checks passed");
    }

    private static void restarts() throws IOException {
        long min = 2_000, max = 60_000, stable = 60_000;
        assert HubConfig.nextBackoff(min, 1_000, false, min, max, stable) == 4_000 : "a quick crash doubles the wait";
        assert HubConfig.nextBackoff(32_000, 1_000, false, min, max, stable) == max : "the wait stops at the cap";
        assert HubConfig.nextBackoff(32_000, 90_000, false, min, max, stable) == min : "a long run starts over";
        assert HubConfig.nextBackoff(32_000, 1_000, true, min, max, stable) == min : "a restart asked for is not delayed";
        Path dir = Files.createTempDirectory("mode");
        Path mode = dir.resolve("mode");
        HubConfig.writeMode(mode, true);
        assert HubConfig.isActive(new String(Files.readAllBytes(mode), StandardCharsets.UTF_8));
        HubConfig.writeMode(mode, false);
        assert !HubConfig.isActive(new String(Files.readAllBytes(mode), StandardCharsets.UTF_8));
        assert !Files.exists(dir.resolve("mode.tmp")) : "no temp file is left";
        Files.delete(mode);
        // A target that cannot be replaced (here a folder with something in it) fails, and leaves no temp file behind.
        Files.createDirectory(mode);
        Files.write(mode.resolve("x"), new byte[] {1});
        boolean failed = false;
        try { HubConfig.writeMode(mode, true); } catch (IOException expected) { failed = true; }
        assert failed : "an unwritable mode is an error";
        assert !Files.exists(dir.resolve("mode.tmp")) : "a failed write leaves no temp file";
        Files.delete(mode.resolve("x"));
        Files.delete(mode);
        Files.delete(dir);
    }

    private static void parsing() {
        Map<String, String> ok = HubConfig.parseSecrets("# note\r\nHQ_RELAY__TELEGRAM_TOKEN=abc:def\r\n\nOPENROUTER_API_KEY=sk-x=y\nDEEPSEEK_API_KEY=\n");
        assert ok.size() == 3 : ok;
        assert ok.get("OPENROUTER_API_KEY").equals("sk-x=y") : "value keeps a second '='";
        assert ok.get("DEEPSEEK_API_KEY").isEmpty() : "empty value means remove";
        assert HubConfig.parseSecrets("").isEmpty();
        assert HubConfig.parseSecrets("\n# only a comment\n").isEmpty();
        assert HubConfig.isCredentialName("OPENROUTER_API_KEY") && HubConfig.isCredentialName("HQ_RELAY__DISCORD_TOKEN");
        for (String name : new String[] {"HQ_MCP_DEV_NO_AUTH", "HQ_WEB_AUTH_TOKEN", "LD_PRELOAD", "", "token", "A_TOKEN\n"}) {
            assert !HubConfig.isCredentialName(name) : name;
        }
        assert HubConfig.isActive("active\n") && HubConfig.isActive("  active ");
        assert !HubConfig.isActive(null) && !HubConfig.isActive("") && !HubConfig.isActive("ACTIVE") && !HubConfig.isActive("standby");
        assert String.join(" ", HubConfig.hqArgs(true, 5678)).equals("start all");
        assert String.join(" ", HubConfig.hqArgs(false, 5678)).equals("web --no-open --supervised --port 5678 --bind 127.0.0.1");
    }

    // Every refusal must name the line and say why, and none may echo the value.
    private static void rejections() {
        for (String name : new String[] {"LD_PRELOAD", "PATH", "HOME", "TMPDIR", "lower_api_key", "X_TOKEN_EXTRA", "_TOKEN",
                "HQ_MCP_DEV_NO_AUTH", "HQ_VAULT_PATH", "HQ_BIN_PATH", "HQ_CONFIG_PATH", "HQ_WEB_BIND", "HQ_RELAY__TELEGRAM_AUTHORIZED_CHAT_ID"}) {
            rejected(name + "=" + MARK, "line 1: name not allowed");
        }
        rejected("HQ_WEB_AUTH_TOKEN=" + MARK, "line 1: name reserved for the app");
        rejected("NOEQUALS" + MARK, "line 1: expected NAME=value");
        rejected("=" + MARK, "line 1: expected NAME=value");
        rejected("A_TOKEN=1\nA_TOKEN=" + MARK, "line 2: name repeated");
        rejected("A_TOKEN=a\u0001" + MARK, "line 1: control character in value");
        rejected("A_TOKEN=" + MARK + "x".repeat(HubConfig.MAX_VALUE_CHARS), "line 1: value too long");
        StringBuilder many = new StringBuilder();
        for (int i = 0; i <= HubConfig.MAX_SECRETS; i++) many.append("N").append(i).append("_TOKEN=").append(MARK).append('\n');
        rejected(many.toString(), "too many names");
    }

    private static void alerts() {
        java.util.List<String> seen = java.util.Arrays.asList("a", "b");
        assert HubConfig.unseen(java.util.Arrays.asList("d", "b", "c", "a", "d"), seen).equals(java.util.Arrays.asList("d", "c")) : "new ids only, in order, once";
        assert HubConfig.unseen(java.util.Arrays.asList("a"), seen).isEmpty() && HubConfig.unseen(java.util.Collections.<String>emptyList(), seen).isEmpty();
        assert HubConfig.remember(seen, java.util.Arrays.asList("c", "a", "d"), 10).equals(java.util.Arrays.asList("a", "b", "c", "d")) : "no repeats, newest last";
        assert HubConfig.remember(java.util.Arrays.asList("1", "2", "3"), java.util.Arrays.asList("4", "5"), 3).equals(java.util.Arrays.asList("3", "4", "5")) : "oldest dropped past the cap";
        assert HubConfig.safePath("/notifications").equals("/notifications") && HubConfig.safePath("/").equals("/") : "plain paths pass";
        for (String bad : new String[] {null, "", "notifications", "/../x", "/a/b", "/Tasks", "//evil.example", "/a?x=1", "/a#t", "/" + "a".repeat(21)}) {
            assert HubConfig.safePath(bad).equals("/") : bad;
        }
    }

    private static void environment() {
        Map<String, String> secrets = new LinkedHashMap<>();
        secrets.put("OPENROUTER_API_KEY", "k");
        secrets.put("HQ_WEB_BIND", "0.0.0.0"); // cannot come from a file, but the merge must hold anyway
        Map<String, String> fixed = Map.of("HQ_WEB_BIND", "127.0.0.1", "HQ_WS_PORT", "5678");
        Map<String, String> env = HubConfig.childEnv(secrets, fixed);
        assert env.get("HQ_WEB_BIND").equals("127.0.0.1") : "fixed settings win";
        assert env.get("OPENROUTER_API_KEY").equals("k") && env.get("HQ_WS_PORT").equals("5678");
    }

    private static void files() throws Exception {
        Path dir = Files.createTempDirectory("hubcheck");
        try {
            Path inbox = dir.resolve("secrets.in");
            Files.write(inbox, "A_TOKEN=abc\n".getBytes(StandardCharsets.UTF_8));
            assert new String(HubConfig.readRegular(inbox, HubConfig.MAX_FILE_BYTES), StandardCharsets.UTF_8).equals("A_TOKEN=abc\n");

            // wipe overwrites the inode before deleting: a second hard link still points at it
            Path alias = dir.resolve("alias");
            Files.createLink(alias, inbox);
            HubConfig.wipe(inbox);
            assert !Files.exists(inbox) : "wiped file is deleted";
            byte[] left = Files.readAllBytes(alias);
            assert left.length == 12 : "length kept";
            for (byte b : left) assert b == 0 : "contents zeroed";

            // a link is unlinked, never written through
            Path target = dir.resolve("target");
            Files.write(target, "KEEP".getBytes(StandardCharsets.UTF_8));
            Files.createSymbolicLink(inbox, target);
            refused(inbox);
            HubConfig.wipe(inbox);
            assert !Files.exists(inbox, java.nio.file.LinkOption.NOFOLLOW_LINKS) : "link removed";
            assert new String(Files.readAllBytes(target), StandardCharsets.UTF_8).equals("KEEP") : "target untouched";

            // exactly at the cap is read; one byte over is refused and deleted
            Files.write(inbox, new byte[(int) HubConfig.MAX_FILE_BYTES]);
            assert HubConfig.readRegular(inbox, HubConfig.MAX_FILE_BYTES).length == HubConfig.MAX_FILE_BYTES : "file at the cap is read whole";
            HubConfig.wipe(inbox);
            assert !Files.exists(inbox);

            Files.write(inbox, new byte[(int) HubConfig.MAX_FILE_BYTES + 1]);
            refused(inbox);
            HubConfig.wipe(inbox);
            assert !Files.exists(inbox);

            // a named pipe would block a reader; it is refused without being opened
            if (new ProcessBuilder("mkfifo", inbox.toString()).start().waitFor() == 0) {
                refused(inbox);
                HubConfig.wipe(inbox);
                assert !Files.exists(inbox) : "pipe removed";
            }

            // a directory is refused; wiping a missing path is quiet
            Files.createDirectory(inbox);
            refused(inbox);
            Files.delete(inbox);
            HubConfig.wipe(inbox);
        } finally {
            try (var walk = Files.walk(dir)) {
                walk.sorted(java.util.Comparator.reverseOrder()).forEach(p -> p.toFile().delete());
            }
        }
    }

    private static void refused(Path p) {
        try {
            HubConfig.readRegular(p, HubConfig.MAX_FILE_BYTES);
        } catch (IOException e) {
            return;
        }
        throw new AssertionError("read a file it should refuse: " + p.getFileName());
    }

    private static void rejected(String text, String message) {
        try {
            HubConfig.parseSecrets(text);
        } catch (IllegalArgumentException e) {
            assert e.getMessage().contains(message) : "wanted '" + message + "', got '" + e.getMessage() + "'";
            assert !e.getMessage().contains(MARK) : "an error must never echo a value: " + e.getMessage();
            return;
        }
        throw new AssertionError("accepted: " + message);
    }
}

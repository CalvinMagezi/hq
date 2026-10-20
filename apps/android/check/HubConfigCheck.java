package example.hq.spike;

import java.util.Map;

/** Run by check.sh with assertions on: java -ea. Exits non-zero on the first failed check. */
public class HubConfigCheck {
    public static void main(String[] args) {
        Map<String, String> ok = HubConfig.parseSecrets("# note\r\nHQ_RELAY__TELEGRAM_TOKEN=abc:def\r\n\nOPENROUTER_API_KEY=sk-x=y\nDEEPSEEK_API_KEY=\n");
        assert ok.size() == 3 : ok;
        assert ok.get("OPENROUTER_API_KEY").equals("sk-x=y") : "value keeps a second '='";
        assert ok.get("DEEPSEEK_API_KEY").isEmpty() : "empty value means remove";
        assert HubConfig.parseSecrets("").isEmpty();
        assert HubConfig.parseSecrets("\n# only a comment\n").isEmpty();

        for (String name : new String[] {"LD_PRELOAD", "PATH", "HOME", "TMPDIR", "lower_api_key", "HQ_", "X_TOKEN_EXTRA", "_TOKEN"}) {
            rejected(name + "=v", "line 1: name not allowed");
        }
        rejected("NOEQUALS", "line 1: expected NAME=value");
        rejected("=value", "line 1: expected NAME=value");
        rejected("HQ_A=1\nHQ_A=2", "line 2: name repeated");
        rejected("HQ_A=a\u0001b", "line 1: control character in value");
        rejected("HQ_A=" + "x".repeat(HubConfig.MAX_VALUE_CHARS + 1), "line 1: value too long");
        StringBuilder many = new StringBuilder();
        for (int i = 0; i <= HubConfig.MAX_SECRETS; i++) many.append("HQ_N").append(i).append("=v\n");
        rejected(many.toString(), "too many names");

        try {
            HubConfig.parseSecrets("HQ_A=sk-SECRETVALUE\nbroken sk-SECRETVALUE");
            throw new AssertionError("should have failed");
        } catch (IllegalArgumentException e) {
            assert !e.getMessage().contains("SECRETVALUE") : "an error must never echo a value";
        }

        assert HubConfig.isActive("active\n") && HubConfig.isActive("  active ");
        assert !HubConfig.isActive(null) && !HubConfig.isActive("") && !HubConfig.isActive("ACTIVE") && !HubConfig.isActive("standby");
        assert String.join(" ", HubConfig.hqArgs(true, 5678)).equals("start all");
        assert String.join(" ", HubConfig.hqArgs(false, 5678)).equals("web --no-open --port 5678 --bind 127.0.0.1");
        System.out.println("HubConfig checks passed");
    }

    private static void rejected(String text, String message) {
        try {
            HubConfig.parseSecrets(text);
        } catch (IllegalArgumentException e) {
            assert e.getMessage().contains(message) : "wanted '" + message + "', got '" + e.getMessage() + "'";
            return;
        }
        throw new AssertionError("accepted: " + message);
    }
}

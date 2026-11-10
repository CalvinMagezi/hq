package example.hq.spike;

import java.util.List;
import java.util.Map;
import org.json.JSONObject;

/** Lets a test in another package read which items Alerts counts as silenced. */
public final class AlertsSilenced {
    private AlertsSilenced() {}

    public static List<String> ids(Map<String, JSONObject> items) {
        return Alerts.silencedIds(items);
    }
}

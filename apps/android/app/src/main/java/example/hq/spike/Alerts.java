package example.hq.spike;

import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.content.Context;
import android.content.SharedPreferences;
import android.util.Log;
import java.io.IOException;
import java.io.InputStream;
import java.net.HttpURLConnection;
import java.net.URL;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.function.BooleanSupplier;
import org.json.JSONArray;
import org.json.JSONException;
import org.json.JSONObject;

/**
 * Turns HQ's unread notifications (approvals waiting on the owner, and notify_owner messages) into
 * Android notifications. HQ's web push needs a browser push service that a WebView does not have, so
 * the app asks its own hq over loopback instead. The first poll after install only records what is
 * already there, so a backlog never arrives as a flood.
 */
final class Alerts {
    private static final long POLL_MS = 20_000;
    private static final int TIMEOUT_MS = 5_000;
    private static final int MAX_PER_POLL = 5;
    private static final int MAX_RESPONSE_BYTES = 1 << 20;
    private static final String PREFS = "alerts";
    private static final String SEEN = "seen";
    private static final String SEP = "\n";

    private Alerts() { }

    static void watch(Context c, BooleanSupplier alive) {
        NotificationManager nm = (NotificationManager) c.getSystemService(Context.NOTIFICATION_SERVICE);
        for (NotificationChannel ch : AlertPoster.channels()) nm.createNotificationChannel(ch);
        nm.deleteNotificationChannel("alerts");
        String lastProblem = "";
        while (alive.getAsBoolean()) {
            try {
                poll(c);
                lastProblem = "";
            } catch (IOException | JSONException | RuntimeException e) {
                String problem = e.getClass().getSimpleName();
                if (!problem.equals(lastProblem)) Log.w(HubService.TAG, "alert poll failed: " + problem);
                lastProblem = problem;
            }
            // Notes and tasks saved while HQ was down go out as soon as it answers.
            try {
                example.hq.spike.capture.OutboxSender.flushNow(c);
            } catch (RuntimeException e) {
                Log.w(HubService.TAG, "outbox flush failed: " + e.getClass().getSimpleName());
            }
            try {
                Thread.sleep(POLL_MS);
            } catch (InterruptedException e) {
                return;
            }
        }
    }

    private static void poll(Context c) throws IOException, JSONException {
        Map<String, JSONObject> items = fetch(c);
        SharedPreferences prefs = c.getSharedPreferences(PREFS, Context.MODE_PRIVATE);
        String stored = prefs.getString(SEEN, null);
        List<String> seen = stored == null || stored.isEmpty() ? new ArrayList<String>() : Arrays.asList(stored.split(SEP));
        List<String> fresh = HubConfig.unseen(new ArrayList<>(items.keySet()), seen);
        if (stored != null) AlertPoster.cancelGone(c, seen, items.keySet());
        if (fresh.isEmpty() && stored != null) return;
        if (stored != null) AlertPoster.post(c, items, fresh, MAX_PER_POLL);
        String joined = String.join(SEP, HubConfig.remember(seen, fresh, HubConfig.SEEN_CAP));
        prefs.edit().putString(SEEN, joined).apply();
    }

    /** Unread items, newest first, keyed by id. */
    private static Map<String, JSONObject> fetch(Context c) throws IOException, JSONException {
        HttpURLConnection h = (HttpURLConnection) new URL("http://127.0.0.1:" + HubService.PORT + "/api/notifications?state=pending&limit=30").openConnection();
        h.setConnectTimeout(TIMEOUT_MS);
        h.setReadTimeout(TIMEOUT_MS);
        h.setRequestProperty("Authorization", "Bearer " + HubService.token(c));
        if (h.getResponseCode() == 401) HubService.recoverToken(c);
        if (h.getResponseCode() != 200) throw new IOException("status " + h.getResponseCode());
        JSONArray arr = new JSONObject(read(h.getInputStream())).getJSONArray("notifications");
        Map<String, JSONObject> out = new LinkedHashMap<>();
        for (int i = 0; i < arr.length(); i++) {
            JSONObject o = arr.getJSONObject(i);
            out.put(o.getString("id"), o);
        }
        return out;
    }

    private static String read(InputStream in) throws IOException {
        try (InputStream s = in) {
            byte[] buf = new byte[MAX_RESPONSE_BYTES];
            int n = 0;
            for (int r; n < buf.length && (r = s.read(buf, n, buf.length - n)) > 0; ) n += r;
            return new String(buf, 0, n, StandardCharsets.UTF_8);
        }
    }
}

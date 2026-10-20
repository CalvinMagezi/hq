package example.hq.spike;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.content.Context;
import android.content.Intent;
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
    static final String CHANNEL = "alerts";
    private static final long POLL_MS = 20_000;
    private static final int TIMEOUT_MS = 5_000;
    private static final int MAX_PER_POLL = 5;
    private static final int MAX_BODY_CHARS = 400;
    private static final int MAX_RESPONSE_BYTES = 1 << 20;
    private static final String PREFS = "alerts";
    private static final String SEEN = "seen";
    private static final String SEP = "\n";
    private static final int ID_BASE = 100;
    private static final int ID_SPAN = 100_000;

    private Alerts() { }

    static void watch(Context c, BooleanSupplier alive) {
        ((NotificationManager) c.getSystemService(Context.NOTIFICATION_SERVICE)).createNotificationChannel(
                channel());
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
            try {
                Thread.sleep(POLL_MS);
            } catch (InterruptedException e) {
                return;
            }
        }
    }

    private static NotificationChannel channel() {
        NotificationChannel ch = new NotificationChannel(CHANNEL, "Alerts", NotificationManager.IMPORTANCE_HIGH);
        ch.setDescription("Approvals and messages from your agents");
        return ch;
    }

    private static void poll(Context c) throws IOException, JSONException {
        Map<String, JSONObject> items = fetch(c);
        SharedPreferences prefs = c.getSharedPreferences(PREFS, Context.MODE_PRIVATE);
        String stored = prefs.getString(SEEN, null);
        List<String> seen = stored == null || stored.isEmpty() ? new ArrayList<String>() : Arrays.asList(stored.split(SEP));
        List<String> fresh = HubConfig.unseen(new ArrayList<>(items.keySet()), seen);
        if (fresh.isEmpty() && stored != null) return;
        if (stored != null) post(c, items, fresh);
        String joined = String.join(SEP, HubConfig.remember(seen, fresh, HubConfig.SEEN_CAP));
        prefs.edit().putString(SEEN, joined).apply();
    }

    /** Unread items, newest first, keyed by id. */
    private static Map<String, JSONObject> fetch(Context c) throws IOException, JSONException {
        HttpURLConnection h = (HttpURLConnection) new URL("http://127.0.0.1:" + HubService.PORT + "/api/notifications?state=pending&limit=30").openConnection();
        h.setConnectTimeout(TIMEOUT_MS);
        h.setReadTimeout(TIMEOUT_MS);
        h.setRequestProperty("Authorization", "Bearer " + HubService.token(c));
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

    /** Oldest first, so the newest ends on top; past the cap one line stands for the rest. */
    private static void post(Context c, Map<String, JSONObject> items, List<String> fresh) {
        NotificationManager nm = (NotificationManager) c.getSystemService(Context.NOTIFICATION_SERVICE);
        List<String> shown = fresh.size() > MAX_PER_POLL ? fresh.subList(0, MAX_PER_POLL) : fresh;
        for (int i = shown.size() - 1; i >= 0; i--) {
            JSONObject o = items.get(shown.get(i));
            String title = o.optString("title", "AgentHQ");
            String text = o.optString("description", "");
            nm.notify(ID_BASE + Math.abs(shown.get(i).hashCode() % ID_SPAN), build(c, title, text));
        }
        if (fresh.size() > shown.size()) {
            nm.notify(ID_BASE - 1, build(c, "AgentHQ", (fresh.size() - shown.size()) + " more unread items"));
        }
    }

    private static Notification build(Context c, String title, String text) {
        String body = text.length() > MAX_BODY_CHARS ? text.substring(0, MAX_BODY_CHARS) + "..." : text;
        Intent open = new Intent(c, MainActivity.class).putExtra(MainActivity.EXTRA_PATH, "/notifications")
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK | Intent.FLAG_ACTIVITY_SINGLE_TOP);
        PendingIntent tap = PendingIntent.getActivity(c, 0, open, PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
        return new Notification.Builder(c, CHANNEL).setSmallIcon(HubService.statIcon(c)).setContentTitle(title)
                .setContentText(body).setStyle(new Notification.BigTextStyle().bigText(body))
                .setContentIntent(tap).setAutoCancel(true).setCategory(Notification.CATEGORY_MESSAGE).build();
    }
}

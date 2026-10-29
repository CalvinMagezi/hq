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
 *
 * When the owner has signed the phone in to their HQ on a server (ServerHq), that HQ is asked too, once
 * a minute and with the ETag of its last answer, so a list that has not changed costs a bodiless 304 on
 * mobile data. Its items keep their own seen list and carry ServerHq.ID_PREFIX, so a notification and its
 * answer go back to the HQ that asked.
 */
final class Alerts {
    private static final long POLL_MS = 20_000;
    private static final int TIMEOUT_MS = 5_000;
    private static final int MAX_PER_POLL = 5;
    private static final int MAX_RESPONSE_BYTES = 1 << 20;
    private static final String PREFS = "alerts";
    private static final String SEEN = "seen";
    private static final String SERVER_SEEN = "server_seen";
    private static final String SERVER_ETAG = "server_etag";
    private static final long SERVER_POLL_MS = 60_000;
    private static final String SEP = "\n";
    private static long lastServerPoll;

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
            try {
                pollServer(c);
            } catch (IOException | JSONException | RuntimeException e) {
                Log.w(HubService.TAG, "server alert poll failed: " + e.getClass().getSimpleName());
            }
            // Notes and tasks saved while HQ was down go out as soon as it answers; the widgets learn what is in progress.
            try {
                example.hq.spike.capture.OutboxSender.flushNow(c);
                example.hq.spike.capture.Widgets.refreshTasks(c);
            } catch (Exception e) {
                // Kotlin does not declare the IOExceptions it throws, so Java sees them only here; nothing in this step may end the poll loop.
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
        String newest = null;
        int needing = 0;
        for (JSONObject item : items.values()) {
            if (!AlertPoster.needsYou(item)) continue;
            if (newest == null) newest = item.optString("title", "");
            needing++;
        }
        try {
            example.hq.spike.capture.Widgets.needs(c, needing, newest);
        } catch (Exception e) {
            Log.w(HubService.TAG, "widget update failed: " + e.getClass().getSimpleName());
        }
        postNew(c, items, SEEN);
    }

    /** Posts what is new in [items] and takes down what is gone, against the seen list kept under [seenKey]. */
    private static void postNew(Context c, Map<String, JSONObject> items, String seenKey) {
        SharedPreferences prefs = c.getSharedPreferences(PREFS, Context.MODE_PRIVATE);
        String stored = prefs.getString(seenKey, null);
        List<String> seen = stored == null || stored.isEmpty() ? new ArrayList<String>() : Arrays.asList(stored.split(SEP));
        List<String> fresh = HubConfig.unseen(new ArrayList<>(items.keySet()), seen);
        if (stored != null) AlertPoster.cancelGone(c, seen, items.keySet());
        if (fresh.isEmpty() && stored != null) return;
        if (stored != null) AlertPoster.post(c, items, fresh, MAX_PER_POLL);
        String joined = String.join(SEP, HubConfig.remember(seen, fresh, HubConfig.SEEN_CAP));
        prefs.edit().putString(seenKey, joined).apply();
    }

    /**
     * Starts the server's lists over: on sign-in with an empty seen list, so the questions already waiting
     * there (at most MAX_PER_POLL of them) are shown at once, and on Disconnect with none.
     */
    static void resetServer(Context c, boolean signedIn) {
        SharedPreferences.Editor edit = c.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().remove(SERVER_ETAG);
        if (signedIn) edit.putString(SERVER_SEEN, "");
        else edit.remove(SERVER_SEEN);
        edit.apply();
        lastServerPoll = 0;
    }

    /** The owner's HQ on a server, at most once a minute; nothing when the phone is not signed in to one. */
    private static void pollServer(Context c) throws IOException, JSONException {
        long now = android.os.SystemClock.elapsedRealtime();
        if (lastServerPoll != 0 && now - lastServerPoll < SERVER_POLL_MS) return;
        lastServerPoll = now;
        ServerHq.Server server = ServerHq.current(c);
        if (server == null) return;
        SharedPreferences prefs = c.getSharedPreferences(PREFS, Context.MODE_PRIVATE);
        ServerHq.Reply reply = ServerHq.call(server.getAddress(), "/api/notifications?state=pending&limit=30", server.getToken(), null, prefs.getString(SERVER_ETAG, null));
        if (reply.status == 304) return;
        if (reply.status == 401) {
            ServerHq.markSignedOut(c);
            prefs.edit().remove(SERVER_ETAG).remove(SERVER_SEEN).apply();
            AlertPoster.serverSignedOut(c, server.getAddress());
            return;
        }
        if (reply.status != 200) throw new IOException("server status " + reply.status);
        postNew(c, itemsOf(reply.body, ServerHq.ID_PREFIX), SERVER_SEEN);
        // Kept only once its list was handled, so a failed pass asks for the whole list again.
        prefs.edit().putString(SERVER_ETAG, reply.etag).apply();
    }

    /** Unread items, newest first, keyed by id. */
    private static Map<String, JSONObject> fetch(Context c) throws IOException, JSONException {
        HttpURLConnection h = (HttpURLConnection) new URL("http://127.0.0.1:" + HubService.PORT + "/api/notifications?state=pending&limit=30").openConnection();
        h.setConnectTimeout(TIMEOUT_MS);
        h.setReadTimeout(TIMEOUT_MS);
        h.setRequestProperty("Authorization", "Bearer " + HubService.token(c));
        if (h.getResponseCode() == 401) HubService.recoverToken(c);
        if (h.getResponseCode() != 200) throw new IOException("status " + h.getResponseCode());
        return itemsOf(read(h.getInputStream()), "");
    }

    /** The items of a notifications reply, newest first, keyed by id with [prefix] put in front of it. */
    private static Map<String, JSONObject> itemsOf(String body, String prefix) throws JSONException {
        JSONArray arr = new JSONObject(body).getJSONArray("notifications");
        Map<String, JSONObject> out = new LinkedHashMap<>();
        for (int i = 0; i < arr.length(); i++) {
            JSONObject o = arr.getJSONObject(i);
            String id = prefix + o.getString("id");
            o.put("id", id);
            out.put(id, o);
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

package example.hq.spike;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.Service;
import android.content.Context;
import android.content.Intent;
import android.content.pm.ServiceInfo;
import android.os.Build;
import android.os.IBinder;
import android.util.Log;
import java.io.File;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicLong;
import java.util.function.Supplier;

/** Keeps `hq` and, when the owner supplied a Caddyfile, Caddy running as children of this app, and turns unread HQ notifications into Android ones. */
public class HubService extends Service {
    static final String TAG = "HQHUB";
    static final String CHANNEL = "hub";
    static final int PORT = 5678;
    private static final int NOTIFICATION_ID = 1;
    private static final long BACKOFF_MIN_MS = 2_000;
    private static final long BACKOFF_MAX_MS = 60_000;
    private static final long STABLE_RUN_MS = 60_000;
    private static final long MAX_LOG_BYTES = 5L << 20;
    private static final long RECOVER_MIN_GAP_MS = 5 * 60 * 1000L;
    private static final String SIGN_IN_MARK = "#token=";
    private static final long MODE_FILE_MAX_BYTES = 64;

    private static final Object LOCK = new Object();
    private static final AtomicBoolean started = new AtomicBoolean(false);
    // A supervisor from before a stop must not outlive it if the service starts again meanwhile.
    private static final AtomicInteger generation = new AtomicInteger();
    private static volatile Process hqProc;
    private static volatile Process caddyProc;

    /** The notification glyph, looked up by name because the build makes no R class. */
    static int statIcon(Context c) {
        int id = c.getResources().getIdentifier("ic_stat_agenthq", "drawable", c.getPackageName());
        return id != 0 ? id : android.R.drawable.stat_notify_sync;
    }

    public static String token(Context c) {
        return SecretStore.webToken(c);
    }

    private static final AtomicLong lastRecovery = new AtomicLong();

    /**
     * The owner chose "Sign out everywhere", which signs out the token this app gave hq as well. Seals a new
     * one and stops hq so its supervisor starts it again with the new token. Done at most once in
     * RECOVER_MIN_GAP_MS, so a 401 with another cause cannot make hq restart over and over.
     */
    public static void recoverToken(Context c) {
        long now = System.currentTimeMillis();
        long last = lastRecovery.get();
        if (now - last < RECOVER_MIN_GAP_MS || !lastRecovery.compareAndSet(last, now)) return;
        try {
            SecretStore.replaceWebToken(c);
            Log.w(TAG, "web token was signed out, replaced it and restarting hq");
            Process p = hqProc;
            if (p != null) p.destroy();
        } catch (IllegalStateException e) {
            Log.e(TAG, "could not replace the web token: " + e.getMessage());
        }
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        getSystemService(NotificationManager.class)
                .createNotificationChannel(new NotificationChannel(CHANNEL, "HQ is running", NotificationManager.IMPORTANCE_LOW));
        Notification n = notification(HubConfig.isActive(readSmall(new File(getFilesDir(), "mode"))));
        try {
            if (Build.VERSION.SDK_INT >= 34) startForeground(NOTIFICATION_ID, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE);
            else startForeground(NOTIFICATION_ID, n);
        } catch (Exception e) {
            Log.e(TAG, "startForeground failed: " + e);
        }
        if (started.compareAndSet(false, true)) {
            int gen = generation.incrementAndGet();
            new Thread(() -> supervise(gen, "hq", this::hqProcess, true), "hq-supervisor").start();
            new Thread(() -> supervise(gen, "caddy", this::caddyProcess, false), "caddy-supervisor").start();
            new Thread(() -> Alerts.watch(getApplicationContext(), () -> gen == generation.get()), "alerts").start();
        }
        return START_STICKY;
    }

    private Notification notification(boolean active) {
        String text = active ? "Answering your chats and running scheduled tasks." : "Only the app is on. Chats and scheduled tasks are paused.";
        return new Notification.Builder(this, CHANNEL).setContentTitle("HQ is running")
                .setContentText(text).setSmallIcon(statIcon(this)).build();
    }

    /** Restarts a child with exponential backoff; a null command means "not configured", so it is skipped. */
    private void supervise(int gen, String name, Supplier<ProcessBuilder> command, boolean isHq) {
        long backoff = BACKOFF_MIN_MS;
        boolean saidUnconfigured = false;
        while (gen == generation.get()) {
            long ranMs = 0;
            try {
                ProcessBuilder pb = command.get();
                if (pb == null) {
                    if (!saidUnconfigured) Log.i(TAG, name + " not configured, waiting for its files");
                    saidUnconfigured = true;
                    if (!pause(gen, BACKOFF_MAX_MS)) return;
                    continue;
                }
                long t0 = System.currentTimeMillis();
                Process p = launch(gen, pb, isHq);
                if (p == null) return;
                Log.i(TAG, name + " started");
                int rc = p.waitFor();
                ranMs = System.currentTimeMillis() - t0;
                Log.w(TAG, name + " exited rc=" + rc + " after " + ranMs / 1000 + " s");
            } catch (InterruptedException e) {
                return;
            } catch (Exception | LinkageError e) {
                Log.e(TAG, name + " failed, will retry: " + e);
            }
            backoff = ranMs >= STABLE_RUN_MS ? BACKOFF_MIN_MS : Math.min(backoff * 2, BACKOFF_MAX_MS);
            if (!pause(gen, backoff)) return;
        }
    }

    /** Starts the child only if no stop happened since the supervisor began; the stop path takes the same lock. */
    private static Process launch(int gen, ProcessBuilder pb, boolean isHq) throws IOException {
        synchronized (LOCK) {
            if (gen != generation.get()) return null;
            Process p = pb.start();
            if (isHq) hqProc = p; else caddyProc = p;
            return p;
        }
    }

    private static boolean pause(int gen, long ms) {
        try {
            Thread.sleep(ms);
        } catch (InterruptedException e) {
            return false;
        }
        return gen == generation.get();
    }

    /**
     * Standby runs the web app only. files/mode containing "active" runs the daemon with the relays
     * and the scheduler, which must never run at the same time as another instance on the same data.
     * Credentials come from the Keystore-backed store and are applied before the fixed settings, so
     * they cannot change the vault, config, bind address or port.
     */
    private ProcessBuilder hqProcess() {
        SecretStore.importPending(this, new File(getFilesDir(), "secrets.in"));
        boolean active = HubConfig.isActive(readSmall(new File(getFilesDir(), "mode")));
        Log.i(TAG, "hq mode: " + (active ? "active (daemon, relays)" : "standby (web only)"));
        getSystemService(NotificationManager.class).notify(NOTIFICATION_ID, notification(active));
        File lib = new File(getApplicationInfo().nativeLibraryDir, "libhq.so");
        File home = new File(getFilesDir(), "hqhome");
        home.mkdirs();
        List<String> cmd = new ArrayList<>();
        cmd.add(lib.getPath());
        cmd.addAll(Arrays.asList(HubConfig.hqArgs(active, PORT)));
        ProcessBuilder pb = new ProcessBuilder(cmd);
        Map<String, String> fixed = new LinkedHashMap<>();
        fixed.put("HOME", home.getPath());
        fixed.put("TMPDIR", getCacheDir().getPath());
        fixed.put("HQ_PROFILE", "mobile");
        fixed.put("HQ_VAULT_PATH", new File(home, ".vault").getPath());
        fixed.put("HQ_CONFIG_PATH", new File(home, "config.yaml").getPath());
        fixed.put("HQ_WEB_STATIC_DIR", new File(getFilesDir(), "web").getPath());
        fixed.put("HQ_WEB_AUTH_TOKEN", token(this));
        fixed.put("HQ_WEB_BIND", "127.0.0.1");
        fixed.put("HQ_WS_PORT", String.valueOf(PORT));
        pb.environment().putAll(HubConfig.childEnv(SecretStore.load(this), fixed));
        pb.redirectErrorStream(true).redirectOutput(ProcessBuilder.Redirect.appendTo(trimmed(new File(getFilesDir(), "hq.log"))));
        return pb;
    }

    private static String readSmall(File f) {
        try {
            return new String(HubConfig.readRegular(f.toPath(), MODE_FILE_MAX_BYTES), StandardCharsets.UTF_8);
        } catch (IOException e) {
            return null;
        }
    }

    /** The owner pushes files/Caddyfile (sites, bind address, certificates); without it Caddy stays off. */
    private ProcessBuilder caddyProcess() {
        File caddyfile = new File(getFilesDir(), "Caddyfile");
        if (!caddyfile.isFile()) return null;
        File lib = new File(getApplicationInfo().nativeLibraryDir, "libcaddy.so");
        File data = new File(getFilesDir(), "caddy");
        data.mkdirs();
        ProcessBuilder pb = new ProcessBuilder(lib.getPath(), "run", "--config", caddyfile.getPath(), "--adapter", "caddyfile");
        pb.directory(getFilesDir());
        Map<String, String> env = pb.environment();
        env.put("HOME", data.getPath());
        env.put("XDG_DATA_HOME", new File(data, "data").getPath());
        env.put("XDG_CONFIG_HOME", new File(data, "config").getPath());
        env.put("TMPDIR", getCacheDir().getPath());
        pb.redirectErrorStream(true).redirectOutput(ProcessBuilder.Redirect.appendTo(trimmed(new File(getFilesDir(), "caddy.log"))));
        return pb;
    }

    /**
     * Children append to their log across restarts, so start over once a log passes the cap. A log that holds a
     * sign-in link (an earlier build printed one at every start) is dropped too, so no token stays on disk in plain text.
     */
    private static File trimmed(File log) {
        if (log.length() > MAX_LOG_BYTES || holdsSignInLink(log)) log.delete();
        return log;
    }

    private static boolean holdsSignInLink(File log) {
        try {
            return new String(HubConfig.readRegular(log.toPath(), MAX_LOG_BYTES), StandardCharsets.ISO_8859_1).contains(SIGN_IN_MARK);
        } catch (IOException e) {
            return false;
        }
    }

    @Override
    public void onDestroy() {
        synchronized (LOCK) {
            generation.incrementAndGet();
            started.set(false);
            if (hqProc != null) hqProc.destroy();
            if (caddyProc != null) caddyProc.destroy();
        }
        super.onDestroy();
    }

    @Override
    public IBinder onBind(Intent i) { return null; }
}

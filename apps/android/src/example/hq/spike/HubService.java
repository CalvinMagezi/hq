package example.hq.spike;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.Service;
import android.content.Context;
import android.content.Intent;
import android.content.SharedPreferences;
import android.content.pm.ServiceInfo;
import android.os.Build;
import android.os.IBinder;
import android.util.Log;
import java.io.File;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.security.SecureRandom;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;
import java.util.Map;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.function.Supplier;

/** Keeps `hq web` and, when the owner supplied a Caddyfile, Caddy running as children of this app. */
public class HubService extends Service {
    static final String TAG = "HQHUB";
    static final String CHANNEL = "hub";
    static final int PORT = 5678;
    private static final long BACKOFF_MIN_MS = 2_000;
    private static final long BACKOFF_MAX_MS = 60_000;
    private static final long STABLE_RUN_MS = 60_000;
    private static final long MAX_LOG_BYTES = 5L << 20;

    private static final AtomicBoolean started = new AtomicBoolean(false);
    // A supervisor from before a stop must not outlive it if the service starts again meanwhile.
    private static final AtomicInteger generation = new AtomicInteger();
    private static volatile Process hqProc;
    private static volatile Process caddyProc;

    static String token(Context c) {
        SharedPreferences p = c.getSharedPreferences("hq", MODE_PRIVATE);
        String t = p.getString("token", null);
        if (t == null) {
            byte[] b = new byte[18];
            new SecureRandom().nextBytes(b);
            StringBuilder s = new StringBuilder("t");
            for (byte x : b) s.append(String.format("%02x", x));
            t = s.toString();
            p.edit().putString("token", t).apply();
        }
        return t;
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        NotificationManager nm = getSystemService(NotificationManager.class);
        nm.createNotificationChannel(new NotificationChannel(CHANNEL, "HQ hub", NotificationManager.IMPORTANCE_LOW));
        Notification n = new Notification.Builder(this, CHANNEL).setContentTitle("HQ hub running")
                .setContentText("Serving this device on 127.0.0.1:" + PORT).setSmallIcon(android.R.drawable.stat_notify_sync).build();
        try {
            if (Build.VERSION.SDK_INT >= 34) startForeground(1, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE);
            else startForeground(1, n);
        } catch (Exception e) {
            Log.e(TAG, "startForeground failed: " + e);
        }
        if (started.compareAndSet(false, true)) {
            int gen = generation.incrementAndGet();
            new Thread(() -> supervise(gen, "hq", this::hqProcess, true), "hq-supervisor").start();
            new Thread(() -> supervise(gen, "caddy", this::caddyProcess, false), "caddy-supervisor").start();
        }
        return START_STICKY;
    }

    /** Restarts a child with exponential backoff; a null command means "not configured", so it is skipped. */
    private void supervise(int gen, String name, Supplier<ProcessBuilder> command, boolean isHq) {
        long backoff = BACKOFF_MIN_MS;
        try {
            while (gen == generation.get()) {
                ProcessBuilder pb = command.get();
                if (pb == null) {
                    Log.i(TAG, name + " not configured, not started");
                    return;
                }
                long t0 = System.currentTimeMillis();
                Process p = pb.start();
                if (isHq) hqProc = p; else caddyProc = p;
                Log.i(TAG, name + " started");
                int rc = p.waitFor();
                long ranMs = System.currentTimeMillis() - t0;
                Log.w(TAG, name + " exited rc=" + rc + " after " + ranMs / 1000 + " s");
                if (gen != generation.get()) return;
                backoff = ranMs >= STABLE_RUN_MS ? BACKOFF_MIN_MS : Math.min(backoff * 2, BACKOFF_MAX_MS);
                Thread.sleep(backoff);
            }
        } catch (Exception e) {
            Log.e(TAG, name + " supervisor failed: " + e);
        }
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
        File lib = new File(getApplicationInfo().nativeLibraryDir, "libhq.so");
        File home = new File(getFilesDir(), "hqhome");
        home.mkdirs();
        List<String> cmd = new ArrayList<>();
        cmd.add(lib.getPath());
        cmd.addAll(Arrays.asList(HubConfig.hqArgs(active, PORT)));
        ProcessBuilder pb = new ProcessBuilder(cmd);
        Map<String, String> env = pb.environment();
        env.putAll(SecretStore.load(this));
        env.put("HOME", home.getPath());
        env.put("TMPDIR", getCacheDir().getPath());
        env.put("HQ_PROFILE", "mobile");
        env.put("HQ_VAULT_PATH", new File(home, ".vault").getPath());
        env.put("HQ_CONFIG_PATH", new File(home, "config.yaml").getPath());
        env.put("HQ_WEB_STATIC_DIR", new File(getFilesDir(), "web").getPath());
        env.put("HQ_WEB_AUTH_TOKEN", token(this));
        env.put("HQ_WEB_BIND", "127.0.0.1");
        env.put("HQ_WS_PORT", String.valueOf(PORT));
        pb.redirectErrorStream(true).redirectOutput(ProcessBuilder.Redirect.appendTo(trimmed(new File(getFilesDir(), "hq.log"))));
        return pb;
    }

    private static String readSmall(File f) {
        try {
            return f.isFile() ? new String(Files.readAllBytes(f.toPath()), StandardCharsets.UTF_8) : null;
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

    /** Children append to their log across restarts, so start over once a log passes the cap. */
    private static File trimmed(File log) {
        if (log.length() > MAX_LOG_BYTES) log.delete();
        return log;
    }

    @Override
    public void onDestroy() {
        generation.incrementAndGet();
        started.set(false);
        if (hqProc != null) hqProc.destroy();
        if (caddyProc != null) caddyProc.destroy();
        super.onDestroy();
    }

    @Override
    public IBinder onBind(Intent i) { return null; }
}

package example.hq.spike;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.Service;
import android.content.Context;
import android.content.Intent;
import android.content.pm.ServiceInfo;
import android.os.Build;
import android.os.Bundle;
import android.os.IBinder;
import android.os.ResultReceiver;
import android.os.SystemClock;
import android.util.Log;
import example.hq.spike.bridge.BridgeSupervisor;
import example.hq.spike.update.Updater;
import java.io.File;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicLong;
import java.util.function.Supplier;

/** Keeps `hq` and, when the owner supplied a Caddyfile, Caddy running as children of this app, and turns unread HQ notifications into Android ones. */
public class HubService extends Service {
    static final String TAG = "HQHUB";
    static final String CHANNEL = "hub";
    public static final int PORT = BuildConfig.HQ_PORT;
    private static final int NOTIFICATION_ID = 1;
    private static final long BACKOFF_MIN_MS = 2_000;
    private static final long BACKOFF_MAX_MS = 60_000;
    private static final long STABLE_RUN_MS = 60_000;
    private static final long FORCE_STOP_AFTER_MS = 20_000;
    private static final long MAX_LOG_BYTES = 5L << 20;
    private static final long RECOVER_MIN_GAP_MS = 5 * 60 * 1000L;
    private static final String SIGN_IN_MARK = "#token=";
    private static final long MODE_FILE_MAX_BYTES = 64;

    private static final Object LOCK = new Object();
    private static final AtomicBoolean started = new AtomicBoolean(false);
    // A supervisor from before a stop must not outlive it if the service starts again meanwhile.
    private static final AtomicInteger generation = new AtomicInteger();
    private static volatile Process hqProc;
    /** hq was stopped on purpose (a new run mode or token), so its exit is not a crash and the next start is not delayed. */
    private static volatile boolean restartAsked;
    /** The run mode the hq child was started with, so a switch that raced a start can be seen and repeated. */
    private static volatile boolean childActive;
    private static volatile Process caddyProc;
    /** While true the supervisor does not start hq: a restore is replacing the vault it would hold open. */
    /** How many callers hold hq stopped (a restore, the moved-phone check, a safe-mode copy); hq starts again at zero. */
    private static final java.util.concurrent.atomic.AtomicInteger holds = new java.util.concurrent.atomic.AtomicInteger();
    private static final java.util.concurrent.atomic.AtomicBoolean copyHolding = new java.util.concurrent.atomic.AtomicBoolean();

    private static boolean held() {
        return holds.get() > 0;
    }
    private static final long HELD_POLL_MS = 1_000;

    /** The notification glyph, looked up by name because the build makes no R class. */
    static int statIcon(Context c) {
        int id = c.getResources().getIdentifier("ic_stat_agenthq", "drawable", c.getPackageName());
        return id != 0 ? id : android.R.drawable.stat_notify_sync;
    }

    public static String token(Context c) {
        return SecretStore.webToken(c);
    }

    private static final AtomicLong lastRecovery = new AtomicLong(-RECOVER_MIN_GAP_MS);
    private static final java.util.concurrent.CopyOnWriteArraySet<Runnable> tokenListeners = new java.util.concurrent.CopyOnWriteArraySet<>();

    /** A visible web page asks to hear when the token is replaced, so it can sign in again with the new one. Each page adds its own and removes only that one. */
    public static void addTokenListener(Runnable r) {
        tokenListeners.add(r);
    }

    public static void removeTokenListener(Runnable r) {
        tokenListeners.remove(r);
    }

    /**
     * The owner chose "Sign out everywhere", which signs out the token this app gave hq as well. Seals a new
     * one and stops hq so its supervisor starts it again with the new token. Done at most once in
     * RECOVER_MIN_GAP_MS, so a 401 with another cause cannot make hq restart over and over.
     */
    public static void recoverToken(Context c) {
        recoverToken(c, false);
    }

    /** [force] is for a deliberate sign-out of everything: the app caused the 401, so the five minute guard against a loop does not apply. */
    public static void recoverToken(Context c, boolean force) {
        long now = SystemClock.elapsedRealtime();
        long last = lastRecovery.get();
        if (!force && (now - last < RECOVER_MIN_GAP_MS || !lastRecovery.compareAndSet(last, now))) return;
        if (force) lastRecovery.set(now);
        try {
            SecretStore.replaceWebToken(c);
            Log.w(TAG, "web token was signed out, replaced it and restarting hq");
            restartHq();
            for (Runnable heard : tokenListeners) {
                try { heard.run(); } catch (RuntimeException e) { Log.w(TAG, "a token listener failed: " + e); }
            }
        } catch (RuntimeException e) {
            lastRecovery.set(last);
            Log.e(TAG, "could not replace the web token: " + e);
        }
    }

    /**
     * Stops the hq child so its supervisor starts it again, which reads files/mode and the token again. Only hq: the
     * service, Caddy and the alerts thread keep running. Callers write the new mode first.
     */
    public static void restartHq() {
        synchronized (LOCK) {
            Process p = hqProc;
            if (p == null || !p.isAlive()) return;
            restartAsked = true;
            p.destroy();
            // hq shuts down in about ten seconds; one that has not gone by then is killed, or the old mode would keep running.
            Thread killer = new Thread(() -> {
                try {
                    if (!p.waitFor(FORCE_STOP_AFTER_MS, TimeUnit.MILLISECONDS)) p.destroyForcibly();
                } catch (InterruptedException ignored) {
                    Thread.currentThread().interrupt();
                }
            }, "hq-stop");
            killer.setDaemon(true);
            killer.start();
        }
    }

    /** Records the owner's choice of run mode and restarts hq so it applies. Throws if the mode file cannot be written, and then nothing restarts. */
    public static void switchMode(Context c, boolean active) throws IOException {
        HubConfig.writeMode(new File(c.getFilesDir(), "mode").toPath(), active);
        restartHq();
    }

    /**
     * Stops hq and keeps it stopped until [releaseHq], so the vault can be replaced. Blocks until the child has
     * ended (at most about 25 s); call it off the main thread. Returns false if it could not be stopped.
     */
    public static boolean holdHq() {
        holds.incrementAndGet();
        restartHq();
        Process p = hqProc;
        if (p == null) return true;
        try {
            if (!p.waitFor(FORCE_STOP_AFTER_MS + 5_000, TimeUnit.MILLISECONDS)) {
                releaseHq();
                return false;
            }
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            releaseHq();
            return false;
        }
        return true;
    }

    /** Gives back one hold; the supervisor starts hq again, with what is stored by then, once none is left. */
    public static void releaseHq() {
        holds.updateAndGet(n -> Math.max(0, n - 1));
    }

    /** Asked by the safe-mode screen, which runs in its own process, to stop hq while the database files are copied. */
    public static final String ACTION_COPY_HOLD = "example.hq.spike.COPY_HOLD";
    public static final String ACTION_COPY_RELEASE = "example.hq.spike.COPY_RELEASE";
    public static final String EXTRA_REPLY = "reply";
    /** The answers sent back to the screen: hq is held, hq was not running so nothing needed holding, hq would not stop. */
    public static final int COPY_HELD = 0;
    public static final int COPY_NOT_RUNNING = 1;
    public static final int COPY_FAILED = 2;
    /** A copy never keeps hq stopped longer than this, whatever happens to the screen. */
    private static final long COPY_HOLD_MAX_MS = 25_000;
    private static Thread copyTimer;

    private void copyHold(Intent intent, int startId) {
        ResultReceiver reply = Build.VERSION.SDK_INT >= 33
                ? intent.getParcelableExtra(EXTRA_REPLY, ResultReceiver.class) : intent.getParcelableExtra(EXTRA_REPLY);
        if (!started.get()) {
            if (reply != null) reply.send(COPY_NOT_RUNNING, Bundle.EMPTY);
            stopSelf(startId);
            return;
        }
        new Thread(() -> {
            // One copy holds at most once; a second request while it runs answers held without another hold.
            boolean ok = copyHolding.get() || holdHq();
            if (ok && copyHolding.compareAndSet(false, true)) {
                Thread timer = new Thread(() -> {
                    try {
                        Thread.sleep(COPY_HOLD_MAX_MS);
                    } catch (InterruptedException e) {
                        // The copy finished first.
                    }
                    releaseCopyHold();
                }, "hq-copy-release");
                synchronized (LOCK) {
                    if (copyTimer != null) copyTimer.interrupt();
                    copyTimer = timer;
                }
                timer.start();
            }
            if (reply != null) reply.send(ok ? COPY_HELD : COPY_FAILED, Bundle.EMPTY);
        }, "hq-copy-hold").start();
    }

    private static void copyRelease() {
        synchronized (LOCK) {
            if (copyTimer != null) copyTimer.interrupt();
            copyTimer = null;
        }
        releaseCopyHold();
    }

    /** The copy's own hold goes once, by the screen or by the timer, never another caller's. */
    private static void releaseCopyHold() {
        if (copyHolding.compareAndSet(true, false)) releaseHq();
    }

    /** True when the running hq child was started in Active mode. */
    public static boolean childActive() {
        return childActive;
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        if (intent != null && ACTION_COPY_HOLD.equals(intent.getAction())) {
            copyHold(intent, startId);
            return START_STICKY;
        }
        if (intent != null && ACTION_COPY_RELEASE.equals(intent.getAction())) {
            copyRelease();
            return START_STICKY;
        }
        NotificationManager nm = getSystemService(NotificationManager.class);
        for (NotificationChannel ch : AlertPoster.channels(this)) nm.createNotificationChannel(ch);
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
            new Thread(() -> Updater.INSTANCE.watch(getApplicationContext(), () -> gen == generation.get()), "update-check").start();
        }
        registerLockReceiver();
        // Does nothing until the owner has turned paired devices on (the pairing screen).
        BridgeSupervisor.INSTANCE.start(getApplicationContext());
        return START_STICKY;
    }

    private android.content.BroadcastReceiver lockReceiver;

    /** The phone locking and unlocking reach only a receiver registered while the service runs. */
    private void registerLockReceiver() {
        if (lockReceiver != null) return;
        lockReceiver = new android.content.BroadcastReceiver() {
            @Override
            public void onReceive(Context context, Intent i) {
                String action = i.getAction();
                Context app = context.getApplicationContext();
                if (Intent.ACTION_SCREEN_OFF.equals(action)) new Thread(() -> Alerts.onLocked(app), "alerts-lock").start();
                else if (Intent.ACTION_USER_PRESENT.equals(action)) new Thread(() -> Alerts.refresh(app), "alerts-unlock").start();
            }
        };
        android.content.IntentFilter filter = new android.content.IntentFilter(Intent.ACTION_SCREEN_OFF);
        filter.addAction(Intent.ACTION_USER_PRESENT);
        if (Build.VERSION.SDK_INT >= 33) registerReceiver(lockReceiver, filter, Context.RECEIVER_NOT_EXPORTED);
        else registerReceiver(lockReceiver, filter);
    }

    private Notification notification(boolean active) {
        String text = active ? "Answering your chats and running scheduled tasks." : "Only the app is on. Chats and scheduled tasks are paused.";
        return new Notification.Builder(this, CHANNEL).setContentTitle("HQ is running")
                .setContentText(text).setSmallIcon(statIcon(this)).build();
    }

    private static final String PREFS = "hub";
    private static final String LAST_START = "last_start";
    private static final String LAST_EXIT = "last_exit";

    private android.content.SharedPreferences hubPrefs() {
        return getSharedPreferences(PREFS, MODE_PRIVATE);
    }

    /** When hq was last launched (epoch millis), or 0 if it never was; what the "HQ did not start" screen reports. */
    public static long lastStart(Context c) {
        return c.getSharedPreferences(PREFS, MODE_PRIVATE).getLong(LAST_START, 0);
    }

    /** The exit code of the hq run that ended last, or null while it has not ended. */
    public static Integer lastExit(Context c) {
        android.content.SharedPreferences p = c.getSharedPreferences(PREFS, MODE_PRIVATE);
        return p.contains(LAST_EXIT) ? p.getInt(LAST_EXIT, 0) : null;
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
                    if (!pause(gen, held() && isHq ? HELD_POLL_MS : BACKOFF_MAX_MS)) return;
                    continue;
                }
                long t0 = System.currentTimeMillis();
                Process p = launch(gen, pb, isHq);
                if (p == null) return;
                Log.i(TAG, name + " started");
                if (isHq) hubPrefs().edit().putLong(LAST_START, System.currentTimeMillis()).remove(LAST_EXIT).apply();
                int rc = p.waitFor();
                ranMs = System.currentTimeMillis() - t0;
                Log.w(TAG, name + " exited rc=" + rc + " after " + ranMs / 1000 + " s");
                if (isHq) hubPrefs().edit().putInt(LAST_EXIT, rc).apply();
            } catch (InterruptedException e) {
                return;
            } catch (Exception | LinkageError e) {
                Log.e(TAG, name + " failed, will retry: " + e);
            }
            boolean asked = isHq && restartAsked;
            if (asked) restartAsked = false;
            backoff = HubConfig.nextBackoff(backoff, ranMs, asked, BACKOFF_MIN_MS, BACKOFF_MAX_MS, STABLE_RUN_MS);
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
        if (held()) return null;
        SecretStore.importPending(this, new File(getFilesDir(), "secrets.in"));
        WebAssets.sync(this);
        boolean active = HubConfig.isActive(readSmall(new File(getFilesDir(), "mode")));
        childActive = active;
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
        Map<String, String> secrets = SecretStore.load(this);
        fixed.putAll(HubConfig.relaySwitches(secrets));
        pb.environment().putAll(HubConfig.childEnv(secrets, fixed));
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
        BridgeSupervisor.INSTANCE.stop();
        if (lockReceiver != null) {
            unregisterReceiver(lockReceiver);
            lockReceiver = null;
        }
        super.onDestroy();
    }

    @Override
    public IBinder onBind(Intent i) { return null; }
}

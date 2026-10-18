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
import java.io.BufferedReader;
import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.InputStreamReader;
import java.security.SecureRandom;
import java.util.Map;

public class HubService extends Service {
    static final String TAG = "HQSPIKE";
    static final String CHANNEL = "hub";
    static final int PORT = 5678;
    private static Process proc;
    private static final java.util.concurrent.atomic.AtomicBoolean launching = new java.util.concurrent.atomic.AtomicBoolean(false);

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
        Log.i(TAG, "service onStartCommand flags=" + flags + " startId=" + startId + " intent=" + (intent == null ? "null (sticky restart)" : "present"));
        NotificationManager nm = getSystemService(NotificationManager.class);
        nm.createNotificationChannel(new NotificationChannel(CHANNEL, "HQ hub", NotificationManager.IMPORTANCE_LOW));
        Notification n = new Notification.Builder(this, CHANNEL).setContentTitle("HQ hub running")
                .setContentText("Serving this device on 127.0.0.1:" + PORT).setSmallIcon(android.R.drawable.stat_notify_sync).build();
        try {
            if (Build.VERSION.SDK_INT >= 34) startForeground(1, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE);
            else startForeground(1, n);
            Log.i(TAG, "startForeground ok");
        } catch (Exception e) {
            Log.e(TAG, "startForeground failed: " + e);
        }
        if (intent != null && intent.hasExtra("load")) {
            int m = intent.getIntExtra("load", 0);
            new Thread(() -> workload(m), "hq-load").start();
        }
        if (intent != null && intent.hasExtra("netprobe")) {
            int m = intent.getIntExtra("netprobe", 0);
            new Thread(() -> netProbe(m), "hq-netprobe").start();
        }
        if ((proc == null || !proc.isAlive()) && launching.compareAndSet(false, true)) {
            Thread t = new Thread(this::launch, "hq-launch");
            t.start();
        }
        return START_STICKY;
    }

    private void launch() {
        try {
            File lib = new File(getApplicationInfo().nativeLibraryDir, "libhq.so");
            File[] listing = new File(getApplicationInfo().nativeLibraryDir).listFiles();
            StringBuilder names = new StringBuilder();
            if (listing != null) for (File f : listing) names.append(f.getName()).append(' ');
            Log.i(TAG, "nativeLibraryDir=" + getApplicationInfo().nativeLibraryDir + " contains: " + names + " exists=" + lib.exists() + " size=" + lib.length());
            Log.i(TAG, "targetSdk=" + getApplicationInfo().targetSdkVersion + " sdk=" + Build.VERSION.SDK_INT);

            // Check 1a: exec from nativeLibraryDir.
            Log.i(TAG, "EXEC nativeLibraryDir: " + run(new ProcessBuilder(lib.getPath(), "--version")));
            // Check 1b: the same file copied to the app's writable directory must be refused.
            File copy = new File(getFilesDir(), "hq-copy");
            try (FileInputStream in = new FileInputStream(lib); FileOutputStream out = new FileOutputStream(copy)) {
                byte[] buf = new byte[1 << 16];
                for (int r; (r = in.read(buf)) > 0; ) out.write(buf, 0, r);
            }
            copy.setExecutable(true, false);
            Log.i(TAG, "EXEC filesDir copy: " + run(new ProcessBuilder(copy.getPath(), "--version")));
            copy.delete();

            File home = new File(getFilesDir(), "hqhome");
            home.mkdirs();
            ProcessBuilder pb = hqProcess("web", "--no-open", "--port", String.valueOf(PORT));
            pb.redirectErrorStream(true).redirectOutput(new File(getFilesDir(), "hq.log"));
            long t0 = System.currentTimeMillis();
            proc = pb.start();
            Log.i(TAG, "hq web started (child of the service process)");
            int rc = proc.waitFor();
            Log.w(TAG, "hq web exited rc=" + rc + " after " + (System.currentTimeMillis() - t0) / 1000 + " s");
        } catch (Exception e) {
            Log.e(TAG, "launch failed: " + e);
        } finally {
            launching.set(false);
        }
    }

    private ProcessBuilder hqProcess(String... args) {
        File lib = new File(getApplicationInfo().nativeLibraryDir, "libhq.so");
        File home = new File(getFilesDir(), "hqhome");
        home.mkdirs();
        String[] cmd = new String[args.length + 1];
        cmd[0] = lib.getPath();
        System.arraycopy(args, 0, cmd, 1, args.length);
        ProcessBuilder pb = new ProcessBuilder(cmd);
        Map<String, String> env = pb.environment();
        env.put("HOME", home.getPath());
        env.put("TMPDIR", getCacheDir().getPath());
        env.put("HQ_PROFILE", "lite");
        env.put("HQ_VAULT_PATH", new File(home, ".vault").getPath());
        env.put("HQ_CONFIG_PATH", new File(home, "config.yaml").getPath());
        env.put("HQ_WEB_STATIC_DIR", new File(getFilesDir(), "web").getPath());
        env.put("HQ_WEB_AUTH_TOKEN", token(this));
        return pb;
    }

    // A mixed workload for the phantom process check: every child is a child of the app process.
    private void workload(int minutes) {
        long end = System.currentTimeMillis() + minutes * 60000L;
        int iteration = 0;
        Log.i(TAG, "LOAD start minutes=" + minutes);
        while (System.currentTimeMillis() < end) {
            long t = System.currentTimeMillis();
            String reindex = run(hqProcess("reindex"));
            long reindexMs = System.currentTimeMillis() - t;
            long searchMs = 0;
            for (String q : new String[] {"tunnel", "memory+schema", "android+kernel", "latency"}) {
                long s0 = System.currentTimeMillis();
                try {
                    java.net.HttpURLConnection c = (java.net.HttpURLConnection) new java.net.URL("http://127.0.0.1:" + PORT + "/api/search?q=" + q).openConnection();
                    c.setRequestProperty("Authorization", "Bearer " + token(this));
                    c.getResponseCode();
                    c.getInputStream().close();
                } catch (Exception e) {
                    Log.w(TAG, "LOAD search failed " + e);
                }
                searchMs += System.currentTimeMillis() - s0;
            }
            iteration++;
            Log.i(TAG, "LOAD it=" + iteration + " reindexMs=" + reindexMs + " searchMs=" + searchMs + " " + reindex);
            try { Thread.sleep(10000); } catch (InterruptedException ignored) { }
        }
        Log.i(TAG, "LOAD done iterations=" + iteration);
    }

    // Outbound network probe for the Doze check: the emulator maps 10.0.2.2 to the host's loopback.
    private void netProbe(int minutes) {
        long end = System.currentTimeMillis() + minutes * 60000L;
        Log.i(TAG, "NETPROBE start minutes=" + minutes);
        while (System.currentTimeMillis() < end) {
            long t = System.currentTimeMillis();
            String result;
            try {
                java.net.HttpURLConnection c = (java.net.HttpURLConnection) new java.net.URL("http://10.0.2.2:8099/ping").openConnection();
                c.setConnectTimeout(3000);
                c.setReadTimeout(3000);
                result = "ok " + c.getResponseCode();
            } catch (Exception e) {
                result = "FAIL " + e.getClass().getSimpleName();
            }
            Log.i(TAG, "NETPROBE " + result + " in " + (System.currentTimeMillis() - t) + " ms");
            try { Thread.sleep(5000); } catch (InterruptedException ignored) { }
        }
    }

    private static String run(ProcessBuilder pb) {
        try {
            pb.redirectErrorStream(true);
            Process p = pb.start();
            BufferedReader r = new BufferedReader(new InputStreamReader(p.getInputStream()));
            String line = r.readLine();
            int rc = p.waitFor();
            return "rc=" + rc + " out=" + line;
        } catch (Exception e) {
            return "FAILED " + e;
        }
    }

    @Override
    public void onDestroy() {
        Log.w(TAG, "service onDestroy");
        if (proc != null) proc.destroy();
        super.onDestroy();
    }

    @Override
    public IBinder onBind(Intent i) { return null; }
}

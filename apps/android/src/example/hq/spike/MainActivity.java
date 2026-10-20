package example.hq.spike;

import android.app.Activity;
import android.content.Intent;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.util.Log;
import android.webkit.WebSettings;
import android.webkit.WebView;
import android.webkit.WebViewClient;
import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.net.HttpURLConnection;
import java.net.URL;

public class MainActivity extends Activity {
    private static final String BASE = "http://127.0.0.1:" + HubService.PORT;
    private static final int HEALTH_POLLS = 600;
    private static final int HEALTH_POLL_MS = 100;
    private WebView web;
    private final Handler ui = new Handler(Looper.getMainLooper());

    @Override
    protected void onCreate(Bundle b) {
        super.onCreate(b);
        try {
            copyAssets("web", new File(getFilesDir(), "web"));
        } catch (Exception e) {
            Log.e(HubService.TAG, "asset copy failed: " + e);
        }
        startForegroundService(new Intent(this, HubService.class));

        web = new WebView(this);
        WebSettings s = web.getSettings();
        s.setJavaScriptEnabled(true);
        s.setDomStorageEnabled(true);
        s.setAllowFileAccess(false);
        web.setWebViewClient(new WebViewClient());
        setContentView(web);
        new Thread(this::waitHealthThenLoad, "health").start();
    }

    private void waitHealthThenLoad() {
        for (int i = 0; i < HEALTH_POLLS; i++) {
            try {
                HttpURLConnection c = (HttpURLConnection) new URL(BASE + "/health").openConnection();
                c.setConnectTimeout(500);
                c.setReadTimeout(500);
                if (c.getResponseCode() == 200) {
                    String url = BASE + "/#token=" + HubService.token(this);
                    ui.post(() -> web.loadUrl(url));
                    return;
                }
            } catch (IOException e) {
                // not up yet
            } catch (IllegalStateException e) {
                Log.e(HubService.TAG, "cannot open the web token: " + e.getMessage());
                return;
            }
            try { Thread.sleep(HEALTH_POLL_MS); } catch (InterruptedException ignored) { }
        }
        Log.e(HubService.TAG, "health never answered");
    }

    private void copyAssets(String from, File to) throws Exception {
        String[] kids = getAssets().list(from);
        if (kids == null || kids.length == 0) {
            to.getParentFile().mkdirs();
            try (InputStream in = getAssets().open(from); FileOutputStream out = new FileOutputStream(to)) {
                byte[] buf = new byte[1 << 16];
                for (int r; (r = in.read(buf)) > 0; ) out.write(buf, 0, r);
            }
            return;
        }
        to.mkdirs();
        for (String k : kids) copyAssets(from + "/" + k, new File(to, k));
    }
}

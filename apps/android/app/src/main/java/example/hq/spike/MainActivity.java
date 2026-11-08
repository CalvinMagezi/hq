package example.hq.spike;

import android.app.Activity;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.os.Build;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.util.Log;
import android.webkit.RenderProcessGoneDetail;
import android.webkit.WebSettings;
import android.webkit.WebView;
import android.webkit.WebViewClient;

public class MainActivity extends Activity {
    private static final String BASE = "http://127.0.0.1:" + HubService.PORT;
    private static final int HEALTH_POLLS = 600;
    private static final int HEALTH_POLL_MS = 100;
    private static final int REAUTH_POLLS = 120;
    private static final int REAUTH_POLL_MS = 500;
    static final String EXTRA_PATH = "path";
    private static final String POST_NOTIFICATIONS = "android.permission.POST_NOTIFICATIONS";
    private static final int REQUEST_NOTIFICATIONS = 1;
    private String path = "/";
    private WebView web;
    private final Handler ui = new Handler(Looper.getMainLooper());

    @Override
    protected void onCreate(Bundle b) {
        super.onCreate(b);
        WebAssets.sync(this);
        startForegroundService(new Intent(this, HubService.class));
        path = HubConfig.safePath(getIntent().getStringExtra(EXTRA_PATH));
        if (Build.VERSION.SDK_INT >= 33 && checkSelfPermission(POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
            requestPermissions(new String[] {POST_NOTIFICATIONS}, REQUEST_NOTIFICATIONS);
        }

        web = new WebView(this);
        WebSettings s = web.getSettings();
        s.setJavaScriptEnabled(true);
        s.setDomStorageEnabled(true);
        s.setAllowFileAccess(false);
        web.setWebViewClient(new WebViewClient() {
            // The renderer shares this process; a crash it caused would end hq too, so the page is built again instead.
            @Override
            public boolean onRenderProcessGone(WebView view, RenderProcessGoneDetail detail) {
                ui.post(MainActivity.this::recreate);
                return true;
            }
        });
        setContentView(web);
        new Thread(this::waitHealthThenLoad, "health").start();
    }

    private final Runnable onNewToken = () -> new Thread(this::reloadWhenNewTokenWorks, "token-reload").start();

    @Override
    protected void onResume() {
        super.onResume();
        HubService.addTokenListener(onNewToken);
    }

    @Override
    protected void onPause() {
        HubService.removeTokenListener(onNewToken);
        super.onPause();
    }

    /**
     * Loads page p signed in, but only from a server that has just proved it is this phone's hq (ServerProof): the page
     * reads the token from its address, and another app may be answering on the port while hq is not. Off the main thread.
     */
    private boolean loadIfProven(String p) {
        String token = HubService.token(this);
        if (!ServerProof.proveNow(BASE, token)) return false;
        String url = BASE + p + "#token=" + token;
        ui.post(() -> web.loadUrl(url));
        return true;
    }

    /** hq is restarted with a replaced token; the page is loaded again only once hq proves it holds that token, since the old process may still be answering. */
    private void reloadWhenNewTokenWorks() {
        for (int i = 0; i < REAUTH_POLLS; i++) {
            try {
                if (loadIfProven(path)) return;
            } catch (IllegalStateException e) {
                // hq is restarting
            }
            try { Thread.sleep(REAUTH_POLL_MS); } catch (InterruptedException ignored) { return; }
        }
        Log.e(HubService.TAG, "hq never accepted the replaced token");
    }

    @Override
    protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        String p = HubConfig.safePath(intent.getStringExtra(EXTRA_PATH));
        new Thread(() -> {
            try {
                if (!loadIfProven(p)) Log.e(HubService.TAG, "hq did not prove itself; the page was not opened");
            } catch (IllegalStateException e) {
                Log.e(HubService.TAG, "cannot open the web token: " + e.getMessage());
            }
        }, "load").start();
    }

    private void waitHealthThenLoad() {
        for (int i = 0; i < HEALTH_POLLS; i++) {
            try {
                if (loadIfProven(path)) return;
            } catch (IllegalStateException e) {
                Log.e(HubService.TAG, "cannot open the web token: " + e.getMessage());
                return;
            }
            try { Thread.sleep(HEALTH_POLL_MS); } catch (InterruptedException ignored) { }
        }
        Log.e(HubService.TAG, "hq never proved itself");
    }
}

package example.hq.spike;

import android.app.Activity;
import android.content.Intent;
import android.os.Build;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.util.Log;
import android.webkit.ConsoleMessage;
import android.webkit.WebChromeClient;
import android.webkit.WebSettings;
import android.webkit.WebView;
import android.webkit.WebViewClient;
import java.io.File;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.net.HttpURLConnection;
import java.net.URL;

public class MainActivity extends Activity {
    static final String TAG = "HQSPIKE";
    private WebView web;
    private final Handler ui = new Handler(Looper.getMainLooper());
    private String host = "127.0.0.1";

    private static final String PROBE = "(async()=>{const r={href:location.href,secure:isSecureContext,push:'PushManager' in window,"
            + "notif:'Notification' in window,sw:'serviceWorker' in navigator,ua:navigator.userAgent};"
            + "try{const regs=await navigator.serviceWorker.getRegistrations();r.swRegs=regs.length;"
            + "const reg=await Promise.race([navigator.serviceWorker.ready,new Promise(res=>setTimeout(()=>res(null),8000))]);r.swReady=!!reg;}"
            + "catch(e){r.swErr=String(e)}"
            + "try{const t=await fetch('/health');r.health=t.status}catch(e){r.fetchErr=String(e)}"
            + "try{r.ls=localStorage.getItem('probe')||'none';localStorage.setItem('probe',String(Date.now()))}catch(e){r.lsErr=String(e)}"
            + "try{const tr=await fetch('/api/ws-ticket',{method:'POST',headers:{Authorization:'Bearer @@TOKEN@@'}});const tj=await tr.json();"
            + "await new Promise(res=>{const ws=new WebSocket('ws://'+location.host+'/ws?ticket='+encodeURIComponent(tj.ticket));"
            + "const to=setTimeout(()=>{r.ws='timeout';res()},5000);ws.onopen=()=>{r.ws='open';clearTimeout(to);ws.close();res()};"
            + "ws.onerror=()=>{r.ws='error';clearTimeout(to);res()}});}catch(e){r.wsErr=String(e)}"
            + "console.log('PROBE '+JSON.stringify(r));})()";

    @Override
    protected void onCreate(Bundle b) {
        super.onCreate(b);
        String h = getIntent().getStringExtra("host");
        if (h != null) host = h;
        long t0 = System.currentTimeMillis();
        Log.i(TAG, "activity onCreate host=" + host);
        try {
            copyAssets("web", new File(getFilesDir(), "web"));
        } catch (Exception e) {
            Log.e(TAG, "asset copy failed: " + e);
        }
        Intent si = new Intent(this, HubService.class);
        for (String k : new String[] {"load", "netprobe"}) {
            if (getIntent().hasExtra(k)) si.putExtra(k, getIntent().getIntExtra(k, 0));
        }
        startForegroundService(si);

        WebView.setWebContentsDebuggingEnabled(true);
        web = new WebView(this);
        WebSettings s = web.getSettings();
        s.setJavaScriptEnabled(true);
        s.setDomStorageEnabled(true);
        s.setAllowFileAccess(false);
        web.setWebChromeClient(new WebChromeClient() {
            @Override
            public boolean onConsoleMessage(ConsoleMessage m) {
                Log.i("HQSPIKE-JS", m.messageLevel() + " " + m.message() + " (" + m.sourceId() + ":" + m.lineNumber() + ")");
                return true;
            }
        });
        web.setWebViewClient(new WebViewClient() {
            @Override
            public void onPageFinished(WebView v, String url) {
                Log.i(TAG, "page finished " + url + " at +" + (System.currentTimeMillis() - t0) + " ms");
                ui.postDelayed(() -> v.evaluateJavascript(PROBE.replace("@@TOKEN@@", HubService.token(MainActivity.this)), null), 6000);
            }

            @Override
            public void onReceivedError(WebView v, android.webkit.WebResourceRequest r, android.webkit.WebResourceError e) {
                Log.e(TAG, "webview error " + e.getErrorCode() + " " + e.getDescription() + " for " + r.getUrl());
            }
        });
        setContentView(web);
        if (getIntent().getBooleanExtra("direct", false)) {
            // Skip the app's own health poll, so the WebView itself meets the cleartext policy.
            ui.postDelayed(() -> web.loadUrl("http://" + host + ":" + HubService.PORT + "/#token=" + HubService.token(this)), 4000);
        } else {
            new Thread(() -> waitHealthThenLoad(t0), "health").start();
        }
    }

    private void waitHealthThenLoad(long t0) {
        String base = "http://" + host + ":" + HubService.PORT;
        for (int i = 0; i < 600; i++) {
            try {
                HttpURLConnection c = (HttpURLConnection) new URL(base + "/health").openConnection();
                c.setConnectTimeout(500);
                c.setReadTimeout(500);
                if (c.getResponseCode() == 200) {
                    Log.i(TAG, "health 200 after " + (System.currentTimeMillis() - t0) + " ms");
                    String url = base + "/#token=" + HubService.token(this);
                    ui.post(() -> web.loadUrl(url));
                    return;
                }
            } catch (Exception e) {
                // not up yet
            }
            try { Thread.sleep(100); } catch (InterruptedException ignored) { }
        }
        Log.e(TAG, "health never answered");
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

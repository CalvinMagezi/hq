package example.hq.spike;

import android.content.Context;
import android.content.SharedPreferences;
import android.content.res.AssetManager;
import android.util.Log;
import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;

/**
 * The web app travels inside the APK, and hq serves it from files/web. Whenever a different build has been installed the folder is
 * replaced, before hq starts (hq reads the page's script hashes for its security policy when it starts), so the app always shows the
 * pages it was built with. A copy that fails leaves the folder that was there.
 */
final class WebAssets {
    private static final String PREFS = "web_assets";
    private static final String STAMP = "stamp";
    private static final String ASSET_DIR = "web";
    private static final int BUFFER_BYTES = 1 << 16;

    private WebAssets() {}

    /** A fresh copy is made when a different build was installed since the last one, or the folder is missing its first page. */
    static boolean needsCopy(long storedStamp, long installedStamp, boolean indexPresent) {
        return storedStamp != installedStamp || !indexPresent;
    }

    static void sync(Context c) {
        try {
            AssetManager assets = c.getAssets();
            String[] bundled = assets.list(ASSET_DIR);
            if (bundled == null || bundled.length == 0) return;
            long installed = c.getPackageManager().getPackageInfo(c.getPackageName(), 0).lastUpdateTime;
            File dir = new File(c.getFilesDir(), ASSET_DIR);
            SharedPreferences prefs = c.getSharedPreferences(PREFS, Context.MODE_PRIVATE);
            if (!needsCopy(prefs.getLong(STAMP, 0L), installed, new File(dir, "index.html").isFile())) return;
            File fresh = new File(c.getFilesDir(), "web.new");
            File old = new File(c.getFilesDir(), "web.old");
            deleteTree(fresh);
            deleteTree(old);
            copy(assets, ASSET_DIR, fresh);
            if (dir.exists() && !dir.renameTo(old)) throw new IOException("could not move the old web folder aside");
            if (!fresh.renameTo(dir)) {
                old.renameTo(dir);
                throw new IOException("could not put the new web folder in place");
            }
            deleteTree(old);
            prefs.edit().putLong(STAMP, installed).apply();
            Log.i(HubService.TAG, "web files updated from this build");
        } catch (Exception e) {
            Log.e(HubService.TAG, "web files not updated: " + e);
        }
    }

    private static void copy(AssetManager assets, String from, File to) throws IOException {
        String[] kids = assets.list(from);
        if (kids == null || kids.length == 0) {
            File parent = to.getParentFile();
            if (parent != null) parent.mkdirs();
            try (InputStream in = assets.open(from); FileOutputStream out = new FileOutputStream(to)) {
                byte[] buf = new byte[BUFFER_BYTES];
                for (int r; (r = in.read(buf)) > 0; ) out.write(buf, 0, r);
            }
            return;
        }
        to.mkdirs();
        for (String k : kids) copy(assets, from + "/" + k, new File(to, k));
    }

    private static void deleteTree(File f) {
        File[] kids = f.listFiles();
        if (kids != null) for (File k : kids) deleteTree(k);
        f.delete();
    }
}

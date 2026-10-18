package example.hq.spike;

import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.util.Log;

public class BootReceiver extends BroadcastReceiver {
    @Override
    public void onReceive(Context c, Intent i) {
        Log.i("HQSPIKE", "BOOT_COMPLETED received, trying to start the specialUse service");
        try {
            c.startForegroundService(new Intent(c, HubService.class));
            Log.i("HQSPIKE", "BOOT start accepted");
        } catch (Exception e) {
            Log.e("HQSPIKE", "BOOT start refused: " + e);
        }
    }
}

package example.hq.spike;

import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.util.Log;

public class BootReceiver extends BroadcastReceiver {
    @Override
    public void onReceive(Context c, Intent i) {
        Log.i(HubService.TAG, "BOOT_COMPLETED received, starting the hub service");
        try {
            c.startForegroundService(new Intent(c, HubService.class));
            Log.i(HubService.TAG, "BOOT start accepted");
        } catch (Exception e) {
            Log.e(HubService.TAG, "BOOT start refused: " + e);
        }
    }
}

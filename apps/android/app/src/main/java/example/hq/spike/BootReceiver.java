package example.hq.spike;

import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.util.Log;

public class BootReceiver extends BroadcastReceiver {
    @Override
    public void onReceive(Context c, Intent i) {
        // MY_PACKAGE_REPLACED too, because an update stops the app and nothing else would start it again.
        Log.i(HubService.TAG, i.getAction() + " received, starting the hub service");
        try {
            c.startForegroundService(new Intent(c, HubService.class));
            Log.i(HubService.TAG, "start accepted");
        } catch (Exception e) {
            Log.e(HubService.TAG, "start refused: " + e);
        }
    }
}

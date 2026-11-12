package example.hq.spike.ui.shell

import android.content.Context
import example.hq.spike.R

/** The words of every confirm dialog, from the catalogue. Each Red action names the title's verb and object. */
object ConfirmCopy {
    private fun red(c: Context, title: String, body: String, keep: String, act: String, busy: Int, extra: String? = null) =
        ConfirmWords(title, body, keep, act, c.getString(busy), ConfirmKind.Red, extra)

    fun stopHelper(c: Context, helper: String) = red(c, c.getString(R.string.stop_helper_title, helper), c.getString(R.string.stop_helper_body), c.getString(R.string.stop_helper_keep), c.getString(R.string.stop_helper_act, helper), R.string.confirm_busy_stopping)

    fun removeDevice(c: Context, device: String) = red(c, c.getString(R.string.remove_device_title, device), c.getString(R.string.remove_device_body), c.getString(R.string.remove_device_keep), c.getString(R.string.remove_device_act), R.string.confirm_busy_removing)

    fun signOutDevice(c: Context, device: String) = red(c, c.getString(R.string.sign_out_device_title, device), c.getString(R.string.sign_out_device_body), c.getString(R.string.sign_out_device_keep), c.getString(R.string.sign_out_device_act), R.string.confirm_busy_signing_out)

    fun signOutAll(c: Context) = red(c, c.getString(R.string.sign_out_all_title), c.getString(R.string.sign_out_all_body), c.getString(R.string.sign_out_all_keep), c.getString(R.string.sign_out_all_act), R.string.confirm_busy_signing_out)

    fun deleteSpeech(c: Context) = red(c, c.getString(R.string.speech_delete_title), c.getString(R.string.speech_delete_body, c.getString(R.string.dl_mb, example.hq.spike.download.DownloadRules.sizeMb(example.hq.spike.voice.VoiceModels.SIZE_BYTES))), c.getString(R.string.speech_delete_keep), c.getString(R.string.speech_delete_act), R.string.confirm_busy_working)

    fun disconnectApp(c: Context, app: String) = red(c, c.getString(R.string.disconnect_app_title, app), c.getString(R.string.disconnect_app_body, app), c.getString(R.string.disconnect_app_keep, app), c.getString(R.string.disconnect_app_act, app), R.string.confirm_busy_disconnecting)

    fun unlinkAccount(c: Context, app: String) = red(c, c.getString(R.string.unlink_account_title, app), c.getString(R.string.unlink_account_body, app), c.getString(R.string.unlink_account_keep), c.getString(R.string.unlink_account_act), R.string.confirm_busy_unlinking)

    fun disconnectServer(c: Context, host: String) = red(c, c.getString(R.string.disconnect_server_title), c.getString(R.string.disconnect_server_body, host), c.getString(R.string.disconnect_server_keep), c.getString(R.string.disconnect_server_act), R.string.confirm_busy_disconnecting)

    /** [only] is true when no other AI key would answer; [unknown] adds that the phone cannot ask HQ about keys set elsewhere. */
    fun removeKey(c: Context, key: String, only: Boolean, unknown: Boolean = false): ConfirmWords =
        if (only) red(c, c.getString(R.string.remove_only_key_title), c.getString(R.string.remove_only_key_body) + if (unknown) " " + c.getString(R.string.remove_only_key_unknown) else "", c.getString(R.string.remove_key_keep), c.getString(R.string.remove_key_act), R.string.confirm_busy_removing, c.getString(R.string.remove_only_key_extra))
        else red(c, c.getString(R.string.remove_key_title, key), c.getString(R.string.remove_key_body), c.getString(R.string.remove_key_keep), c.getString(R.string.remove_key_act), R.string.confirm_busy_removing)

    /** Forgetting what HQ remembers about its suggestions cannot be taken back, so it is a red confirm. */
    fun clearOffers(c: Context) = red(c, c.getString(R.string.settings_suggests_clear_title), c.getString(R.string.settings_suggests_clear_body), c.getString(R.string.settings_suggests_clear_keep), c.getString(R.string.settings_suggests_clear_confirm), R.string.confirm_busy_working)

    fun leaveNote(c: Context, note: String) = red(c, c.getString(R.string.notes_save_your_changes), c.getString(R.string.leave_note_body, note), c.getString(R.string.notes_keep_editing), c.getString(R.string.notes_discard_changes), R.string.confirm_busy_working, c.getString(R.string.leave_note_save))

    private fun ask(c: Context, title: String, body: String, keep: String, act: String) = ConfirmWords(title, body, keep, act, c.getString(R.string.confirm_busy_working), ConfirmKind.Ask)

    fun bulkDone(c: Context, n: Int) = ask(c, c.getString(R.string.bulk_done_title, n.toString()), c.getString(R.string.bulk_done_body), c.getString(R.string.bulk_done_keep), c.getString(R.string.bulk_done_act, n.toString()))

    fun restart(c: Context, title: String, body: String) = ask(c, title, body, c.getString(R.string.confirm_not_now), c.getString(R.string.restart_act))

    fun pause(c: Context, title: String, body: String) = ask(c, title, body, c.getString(R.string.mode_pause_keep), c.getString(R.string.mode_pause_act))

    fun start(c: Context, title: String, body: String) = ask(c, title, body, c.getString(R.string.confirm_not_now), c.getString(R.string.mode_start_act))

    /** The host of a server address for the body, or the address as typed when it has none. */
    fun hostOf(address: String?): String = address.orEmpty().let { a -> runCatching { java.net.URI(a).host }.getOrNull() ?: a }

    /** "OPENROUTER_API_KEY" as the person reads it: "OpenRouter". */
    fun keyLabel(name: String): String = example.hq.spike.ui.firstrun.KeyNames.display(name)
}

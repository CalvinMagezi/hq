package example.hq.spike.voice

import android.Manifest
import android.app.KeyguardManager
import android.app.admin.DevicePolicyManager
import android.app.role.RoleManager
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.provider.Settings
import android.service.voice.VoiceInteractionService
import androidx.core.content.ContextCompat

/** What Android says about HQ as the phone's assistant. Read live, never from a saved flag: only the owner can change it, in Android's own settings. */
object AssistantRole {
    private const val PREFS = "voice"
    private const val KEY_MIC_ASKED = "mic_asked"

    /** `isActiveService` is API 21, so it answers on every version; RoleManager (API 29) only adds an availability read. */
    fun held(c: Context): Boolean = VoiceInteractionService.isActiveService(c, ComponentName(c, AssistantService::class.java))

    /** Whether this phone has an assistant page at all (an Android Go image may not, and a maker may keep the slot). */
    fun available(c: Context): Boolean {
        if (!settingsTries(c).take(1).any { c.packageManager.resolveActivity(it, 0) != null }) return false
        return Build.VERSION.SDK_INT < Build.VERSION_CODES.Q || roleAvailable(c)
    }

    private fun roleAvailable(c: Context): Boolean =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) c.getSystemService(RoleManager::class.java)?.isRoleAvailable(RoleManager.ROLE_ASSISTANT) ?: true else true

    /** A PIN, pattern or password; a swipe is not a lock. */
    fun deviceSecure(c: Context): Boolean = c.getSystemService(KeyguardManager::class.java)?.isDeviceSecure ?: false

    /** The pages that set the assistant, tried in order until one opens. */
    fun settingsTries(c: Context): List<Intent> = listOf(
        Intent(Settings.ACTION_VOICE_INPUT_SETTINGS),
        Intent(Settings.ACTION_MANAGE_DEFAULT_APPS_SETTINGS),
        Intent(Settings.ACTION_SETTINGS),
    )

    /** Android's own screen lock chooser, then the security page. */
    fun screenLockTries(): List<Intent> = listOf(Intent(DevicePolicyManager.ACTION_SET_NEW_PASSWORD), Intent(Settings.ACTION_SECURITY_SETTINGS))

    /** AgentHQ's App info page: no intent opens the microphone permission itself, so it is two taps further. */
    fun appInfo(c: Context): Intent = Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.parse("package:${c.packageName}"))

    fun micAllowed(c: Context): Boolean = ContextCompat.checkSelfPermission(c, Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED

    /** Only that the dialog was shown once, so "not asked yet" can be told from "off in Android". Not a count of refusals. */
    fun markMicAsked(c: Context) = c.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().putBoolean(KEY_MIC_ASKED, true).apply()

    fun micState(c: Context): MicFact = when {
        micAllowed(c) -> MicFact.Allowed
        c.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getBoolean(KEY_MIC_ASKED, false) -> MicFact.Off
        else -> MicFact.NotAsked
    }
}

enum class MicFact { Allowed, NotAsked, Off }

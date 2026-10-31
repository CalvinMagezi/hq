package example.hq.spike

import android.content.Context

/**
 * What the screens may do with the sealed credentials: list the names, seal one, forget one. The rules are
 * [HubConfig]'s, the same as for secrets.in, and no call here ever returns a value.
 */
object KeyVault {
    /** Why a name and value cannot be stored, or null when they can. An empty value is refused here. */
    fun problem(name: String, value: String): String? = if (value.isEmpty()) "value is empty" else HubConfig.entryProblem(name, value)

    fun isName(name: String): Boolean = HubConfig.isCredentialName(name)

    /** The stored names, sorted. Opens the Keystore, so call it off the main thread. */
    fun names(context: Context): List<String> = SecretStore.names(context)

    /** True when any AI service key is stored: a name ending in _API_KEY. */
    fun hasModelKey(context: Context): Boolean = hasModelKey(names(context))

    fun hasModelKey(names: List<String>): Boolean = names.any { it.endsWith("_API_KEY") }

    /** Seals one credential. Returns the reason it was refused, or null when it is stored; hq has to restart to read it. */
    fun put(context: Context, name: String, value: String): String? = try {
        SecretStore.put(context, name, value)
        null
    } catch (e: IllegalArgumentException) {
        e.message ?: "refused"
    } catch (e: IllegalStateException) {
        e.message ?: "not stored"
    }

    fun remove(context: Context, name: String): Boolean = try {
        SecretStore.remove(context, name)
        true
    } catch (e: IllegalStateException) {
        false
    }
}

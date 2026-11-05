package example.hq.spike.download

import android.content.Context
import java.security.MessageDigest

/** Where the answers to "Is this Wi-Fi paid by the MB?" are kept, by fingerprint: true is paid. */
interface AnswerStore {
    fun all(): Map<String, Boolean>
    fun replace(answers: Map<String, Boolean>)
}

class PrefsAnswers(context: Context) : AnswerStore {
    private val prefs = context.applicationContext.getSharedPreferences("paid_wifi", Context.MODE_PRIVATE)

    @Suppress("UNCHECKED_CAST")
    override fun all(): Map<String, Boolean> = prefs.all.filterValues { it is Boolean } as Map<String, Boolean>

    override fun replace(answers: Map<String, Boolean>) {
        prefs.edit().clear().apply { answers.forEach { (k, v) -> putBoolean(k, v) } }.apply()
    }
}

/**
 * What the owner said about each Wi-Fi. A Wi-Fi's name needs a location permission, which HQ never asks for, so a Wi-Fi is
 * told apart by a fingerprint of what Android gives freely: its gateway, DNS servers and search domain.
 */
class PaidWifi(private val store: AnswerStore) {
    /** True for paid, false for free, null for a Wi-Fi HQ has not seen (or one it cannot tell apart). */
    fun answer(fingerprint: String?): Boolean? = fingerprint?.let { store.all()[it] }

    fun remember(fingerprint: String?, paid: Boolean) {
        if (fingerprint != null) store.replace(store.all() + (fingerprint to paid))
    }

    fun free() = store.all().count { !it.value }
    fun paid() = store.all().count { it.value }

    /** Clears every answer and returns them so Undo can put them back. */
    fun forget(): Map<String, Boolean> = store.all().also { store.replace(emptyMap()) }

    fun restore(answers: Map<String, Boolean>) = store.replace(answers)

    companion object {
        /** The same Wi-Fi gives the same key; null when Android gave nothing to tell it apart by, so HQ asks again. */
        fun fingerprint(gateway: String?, dns: List<String>, domains: String?): String? {
            if (gateway.isNullOrBlank() && dns.isEmpty()) return null
            val text = listOf(gateway.orEmpty(), dns.sorted().joinToString(","), domains.orEmpty()).joinToString("|")
            return MessageDigest.getInstance("SHA-256").digest(text.toByteArray()).joinToString("") { "%02x".format(it) }.take(32)
        }
    }
}

package example.hq.spike.contacts

import android.Manifest
import android.content.ContentResolver
import android.content.Context
import android.content.pm.PackageManager
import android.os.Bundle
import android.provider.ContactsContract.CommonDataKinds.Email
import android.provider.ContactsContract.CommonDataKinds.Phone
import android.provider.ContactsContract.Contacts

/** The real Contacts Provider. Only the columns the contract names are queried; callers use a background thread. */
class AndroidContactsReader(context: Context) : ContactsReader {
    private val app = context.applicationContext

    override fun hasPermission(): Boolean = hasPermission(app)

    override fun findByName(pattern: String, limit: Int): List<ContactHit> = guarded {
        val args = Bundle().apply {
            putString(ContentResolver.QUERY_ARG_SQL_SELECTION, "${Contacts.DISPLAY_NAME_PRIMARY} LIKE ? ESCAPE '\\'")
            putStringArray(ContentResolver.QUERY_ARG_SQL_SELECTION_ARGS, arrayOf(pattern))
            putString(ContentResolver.QUERY_ARG_SQL_SORT_ORDER, "${Contacts.DISPLAY_NAME_PRIMARY} ASC, ${Contacts._ID} ASC")
            putInt(ContentResolver.QUERY_ARG_LIMIT, limit)
        }
        val out = mutableListOf<ContactHit>()
        app.contentResolver.query(Contacts.CONTENT_URI, arrayOf(Contacts._ID, Contacts.DISPLAY_NAME_PRIMARY), args, null)?.use { c ->
            while (c.moveToNext()) out += ContactHit(c.getLong(0), c.getString(1).orEmpty())
        }
        out
    }

    override fun phones(contactIds: List<Long>): Map<Long, List<RawPhone>> = guarded {
        val out = LinkedHashMap<Long, MutableList<RawPhone>>()
        app.contentResolver.query(
            Phone.CONTENT_URI,
            arrayOf(Phone.CONTACT_ID, Phone.NUMBER, Phone.TYPE, Phone.LABEL),
            "${Phone.CONTACT_ID} IN (${placeholders(contactIds)})",
            ids(contactIds),
            "${Phone.CONTACT_ID} ASC, ${Phone.IS_PRIMARY} DESC, ${Phone._ID} ASC",
        )?.use { c ->
            while (c.moveToNext()) {
                val label = Phone.getTypeLabel(app.resources, c.getInt(2), c.getString(3)).toString()
                out.getOrPut(c.getLong(0)) { mutableListOf() } += RawPhone(label, c.getString(1).orEmpty())
            }
        }
        out
    }

    override fun emails(contactIds: List<Long>): Map<Long, List<RawEmail>> = guarded {
        val out = LinkedHashMap<Long, MutableList<RawEmail>>()
        app.contentResolver.query(
            Email.CONTENT_URI,
            arrayOf(Email.CONTACT_ID, Email.ADDRESS, Email.TYPE, Email.LABEL),
            "${Email.CONTACT_ID} IN (${placeholders(contactIds)})",
            ids(contactIds),
            "${Email.CONTACT_ID} ASC, ${Email.IS_PRIMARY} DESC, ${Email._ID} ASC",
        )?.use { c ->
            while (c.moveToNext()) {
                val label = Email.getTypeLabel(app.resources, c.getInt(2), c.getString(3)).toString()
                out.getOrPut(c.getLong(0)) { mutableListOf() } += RawEmail(label, c.getString(1).orEmpty())
            }
        }
        out
    }

    private fun placeholders(ids: List<Long>) = ids.joinToString(",") { "?" }
    private fun ids(ids: List<Long>) = ids.map { it.toString() }.toTypedArray()

    private fun <T> guarded(block: () -> T): T = try {
        block()
    } catch (e: SecurityException) {
        throw ContactsStoreException("read not allowed", e)
    } catch (e: IllegalArgumentException) {
        throw ContactsStoreException("read rejected", e)
    }

    companion object {
        fun hasPermission(c: Context): Boolean = c.checkSelfPermission(Manifest.permission.READ_CONTACTS) == PackageManager.PERMISSION_GRANTED
    }
}

package com.omar.smsmanager.data

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.net.Uri
import android.provider.ContactsContract
import java.util.concurrent.ConcurrentHashMap

data class ContactHit(val name: String, val number: String)

object Contacts {
    private class Entry(val name: String?, val at: Long)

    private val cache = ConcurrentHashMap<String, Entry>()
    private const val TTL_MS = 5 * 60 * 1000L

    fun hasPermission(ctx: Context) =
        ctx.checkSelfPermission(Manifest.permission.READ_CONTACTS) == PackageManager.PERMISSION_GRANTED

    fun name(ctx: Context, address: String): String? {
        if (!hasPermission(ctx) || address.isBlank()) return null
        val now = System.currentTimeMillis()
        cache[address]?.takeIf { now - it.at < TTL_MS }?.let { return it.name }
        val name = lookup(ctx, address)
        cache[address] = Entry(name, now)
        return name
    }

    /** Whatever name is cached for [address], however old, without querying. Null if never looked up. */
    fun peek(address: String): String? = cache[address]?.name

    fun isContact(ctx: Context, address: String) = name(ctx, address) != null

    fun clearCache() = cache.clear()

    private fun lookup(ctx: Context, address: String): String? = try {
        val uri = Uri.withAppendedPath(ContactsContract.PhoneLookup.CONTENT_FILTER_URI, Uri.encode(address))
        ctx.contentResolver.query(uri, arrayOf(ContactsContract.PhoneLookup.DISPLAY_NAME), null, null, null)
            ?.use { if (it.moveToFirst()) it.getString(0) else null }
    } catch (_: Exception) {
        null
    }

    fun search(ctx: Context, query: String, limit: Int = 6): List<ContactHit> {
        if (!hasPermission(ctx) || query.isBlank()) return emptyList()
        return try {
            val uri = Uri.withAppendedPath(
                ContactsContract.CommonDataKinds.Phone.CONTENT_FILTER_URI, Uri.encode(query),
            )
            val cols = arrayOf(
                ContactsContract.CommonDataKinds.Phone.DISPLAY_NAME,
                ContactsContract.CommonDataKinds.Phone.NUMBER,
            )
            ctx.contentResolver.query(uri, cols, null, null, null)?.use { c ->
                buildList {
                    while (c.moveToNext() && size < limit) add(ContactHit(c.getString(0).orEmpty(), c.getString(1).orEmpty()))
                }
            } ?: emptyList()
        } catch (_: Exception) {
            emptyList()
        }
    }
}

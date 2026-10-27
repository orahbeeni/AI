package com.omar.smsmanager.data

import android.content.ContentUris
import android.content.ContentValues
import android.content.Context
import android.net.Uri
import android.provider.Telephony.Sms
import android.telephony.PhoneNumberUtils
import com.omar.smsmanager.spam.PatternMatcher

/** Reads and writes the system SMS provider. Writing only works while we are the default SMS app. */
object SmsStore {
    private class Acc(val threadId: Long, val address: String, val snippet: String, val date: Long, val type: Int) {
        var unread = 0
    }

    fun conversations(ctx: Context): List<Conversation> {
        val out = LinkedHashMap<Long, Acc>()
        try {
            val cols = arrayOf(Sms.THREAD_ID, Sms.ADDRESS, Sms.BODY, Sms.DATE, Sms.TYPE, Sms.READ)
            ctx.contentResolver.query(Sms.CONTENT_URI, cols, null, null, "${Sms.DATE} DESC")?.use { c ->
                while (c.moveToNext()) {
                    val type = c.getInt(4)
                    if (type == Sms.MESSAGE_TYPE_DRAFT) continue
                    val acc = out.getOrPut(c.getLong(0)) {
                        Acc(c.getLong(0), c.getString(1).orEmpty(), c.getString(2).orEmpty(), c.getLong(3), type)
                    }
                    if (type == Sms.MESSAGE_TYPE_INBOX && c.getInt(5) == 0) acc.unread++
                }
            }
        } catch (_: SecurityException) {
            return emptyList()
        }
        return out.values.map {
            Conversation(it.threadId, it.address, it.snippet, it.date, it.unread, it.type != Sms.MESSAGE_TYPE_INBOX)
        }
    }

    fun messages(ctx: Context, threadId: Long): List<Message> = try {
        val cols = arrayOf(Sms._ID, Sms.ADDRESS, Sms.BODY, Sms.DATE, Sms.TYPE, Sms.READ)
        ctx.contentResolver.query(Sms.CONTENT_URI, cols, "${Sms.THREAD_ID}=?", arrayOf(threadId.toString()), "${Sms.DATE} ASC")
            ?.use { c ->
                buildList {
                    while (c.moveToNext()) {
                        val type = c.getInt(4)
                        if (type == Sms.MESSAGE_TYPE_DRAFT) continue
                        add(Message(c.getLong(0), c.getString(1).orEmpty(), c.getString(2).orEmpty(), c.getLong(3), type, c.getInt(5) != 0))
                    }
                }
            } ?: emptyList()
    } catch (_: SecurityException) {
        emptyList()
    }

    class Hit(val threadId: Long, val address: String, val body: String, val date: Long)

    /** Searches message text. `*` and `?` in [query] work as wildcards, as they do in rules. */
    fun search(ctx: Context, query: String, limit: Int = 100): List<Hit> {
        val wildcard = PatternMatcher.hasWildcards(query)
        val matches = PatternMatcher.searchMatcher(query)
        val escaped = if (wildcard) PatternMatcher.wildcardToLike(query)
        else query.replace("\\", "\\\\").replace("%", "\\%").replace("_", "\\_")
        return try {
            val cols = arrayOf(Sms.THREAD_ID, Sms.ADDRESS, Sms.BODY, Sms.DATE)
            ctx.contentResolver.query(
                Sms.CONTENT_URI, cols, "${Sms.BODY} LIKE ? ESCAPE '\\'", arrayOf("%$escaped%"), "${Sms.DATE} DESC",
            )?.use { c ->
                buildList {
                    while (c.moveToNext() && size < limit) {
                        val body = c.getString(2).orEmpty()
                        if (wildcard && !matches(body)) continue // LIKE is only a prefilter for wildcard queries
                        add(Hit(c.getLong(0), c.getString(1).orEmpty(), body, c.getLong(3)))
                    }
                }
            } ?: emptyList()
        } catch (_: SecurityException) {
            emptyList()
        }
    }

    /** The existing thread for [address], or 0 when there is none yet. */
    fun findThread(ctx: Context, address: String): Long {
        try {
            ctx.contentResolver.query(
                Sms.CONTENT_URI, arrayOf(Sms.THREAD_ID), "${Sms.ADDRESS}=?", arrayOf(address), "${Sms.DATE} DESC",
            )?.use { if (it.moveToFirst()) return it.getLong(0) }
        } catch (_: SecurityException) {
            return 0
        }
        return conversations(ctx).firstOrNull { PhoneNumberUtils.compare(it.address, address) }?.threadId ?: 0
    }

    fun insertInbox(ctx: Context, address: String, body: String, sentAt: Long, subscription: Int, read: Boolean): Uri? {
        val v = ContentValues().apply {
            put(Sms.ADDRESS, address)
            put(Sms.BODY, body)
            put(Sms.DATE, System.currentTimeMillis())
            put(Sms.DATE_SENT, sentAt)
            put(Sms.READ, if (read) 1 else 0)
            put(Sms.SEEN, if (read) 1 else 0)
            if (subscription >= 0) put(Sms.SUBSCRIPTION_ID, subscription)
        }
        return try {
            ctx.contentResolver.insert(Sms.Inbox.CONTENT_URI, v)
        } catch (_: SecurityException) {
            null
        }
    }

    fun threadIdOf(ctx: Context, uri: Uri): Long = try {
        ctx.contentResolver.query(uri, arrayOf(Sms.THREAD_ID), null, null, null)?.use { if (it.moveToFirst()) it.getLong(0) else 0 } ?: 0
    } catch (_: Exception) {
        0
    }

    fun markRead(ctx: Context, threadId: Long) {
        val v = ContentValues().apply { put(Sms.READ, 1); put(Sms.SEEN, 1) }
        try {
            ctx.contentResolver.update(Sms.CONTENT_URI, v, "${Sms.THREAD_ID}=? AND ${Sms.READ}=0", arrayOf(threadId.toString()))
        } catch (_: SecurityException) {
        }
    }

    fun deleteThread(ctx: Context, threadId: Long) {
        try {
            ctx.contentResolver.delete(Sms.CONTENT_URI, "${Sms.THREAD_ID}=?", arrayOf(threadId.toString()))
        } catch (_: SecurityException) {
        }
    }

    fun deleteMessage(ctx: Context, id: Long) {
        try {
            ctx.contentResolver.delete(ContentUris.withAppendedId(Sms.CONTENT_URI, id), null, null)
        } catch (_: SecurityException) {
        }
    }
}

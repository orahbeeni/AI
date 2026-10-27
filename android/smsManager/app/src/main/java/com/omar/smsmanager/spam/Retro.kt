package com.omar.smsmanager.spam

import android.content.Context
import android.provider.Telephony.Sms
import com.omar.smsmanager.data.Contacts
import com.omar.smsmanager.data.Db
import com.omar.smsmanager.data.Settings
import com.omar.smsmanager.data.Ticker

enum class ScanRange(val label: String, val days: Int) {
    WEEK("1 week", 7),
    MONTH("1 month", 30),
    YEAR("1 year", 365),
    YEARS_5("5 years", 5 * 365),
    YEARS_10("10 years", 10 * 365),
    ALL("All time", 0),
}

class Candidate(val smsId: Long, val sender: String, val body: String, val date: Long, val rule: Rule)

/** Runs the current rules over messages already in the inbox. */
object Retro {
    class Outcome(val moved: Int, val failed: Boolean)

    /** Read-only: lists received messages in [range] that the rules would have blocked. */
    fun scan(ctx: Context, range: ScanRange): List<Candidate> {
        val since = if (range.days == 0) 0L else System.currentTimeMillis() - range.days * Settings.DAY_MS
        val filter = SpamFilter(Db.get(ctx).rules())
        val contactsAllowed = Settings(ctx).contactsAlwaysAllowed
        val isContact = HashMap<String, Boolean>()
        val out = ArrayList<Candidate>()
        try {
            ctx.contentResolver.query(
                Sms.CONTENT_URI,
                arrayOf(Sms._ID, Sms.ADDRESS, Sms.BODY, Sms.DATE),
                "${Sms.TYPE}=? AND ${Sms.DATE}>=?",
                arrayOf(Sms.MESSAGE_TYPE_INBOX.toString(), since.toString()),
                "${Sms.DATE} DESC",
            )?.use { c ->
                while (c.moveToNext()) {
                    val sender = c.getString(1).orEmpty()
                    val body = c.getString(2).orEmpty()
                    val contact = contactsAllowed && isContact.getOrPut(sender) { Contacts.isContact(ctx, sender) }
                    val v = filter.evaluate(sender, body, contact)
                    if (v is Verdict.Blocked) out.add(Candidate(c.getLong(0), sender, body, c.getLong(3), v.rule))
                }
            }
        } catch (_: SecurityException) {
        }
        return out
    }

    /**
     * Copies each message into Blocked, then deletes it from the inbox. The copy always comes first so a
     * failure can never lose a message. Needs the default-SMS role, since only that app may delete.
     */
    fun apply(ctx: Context, items: List<Candidate>): Outcome {
        val db = Db.get(ctx)
        var moved = 0
        var failed = false
        try {
            for (chunk in items.chunked(200)) {
                val copies = chunk.map { db.addBlocked(it.sender, it.body, it.date, it.rule, notify = false) }
                val deleted = try {
                    ctx.contentResolver.delete(Sms.CONTENT_URI, "${Sms._ID} IN (${chunk.joinToString(",") { it.smsId.toString() }})", null)
                } catch (_: SecurityException) {
                    0
                }
                if (deleted == 0) {
                    db.deleteBlockedIds(copies) // nothing left the inbox, so undo the copies
                    failed = true
                    break
                }
                moved += chunk.size
                chunk.groupingBy { it.rule.id }.eachCount().forEach { (ruleId, n) -> db.addHits(ruleId, n) }
                if (deleted != chunk.size) { failed = true; break } // partial: keep every copy rather than risk losing any
            }
        } finally {
            Ticker.bump()
        }
        return Outcome(moved, failed)
    }
}

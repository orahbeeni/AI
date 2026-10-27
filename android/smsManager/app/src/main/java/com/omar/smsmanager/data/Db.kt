package com.omar.smsmanager.data

import android.content.ContentValues
import android.content.Context
import android.database.Cursor
import android.database.sqlite.SQLiteDatabase
import android.database.sqlite.SQLiteOpenHelper
import com.omar.smsmanager.spam.MatchMode
import com.omar.smsmanager.spam.Rule
import com.omar.smsmanager.spam.RuleAction
import com.omar.smsmanager.spam.RuleTarget

/** Our own storage: block/allow rules and the quarantine of blocked messages. */
class Db private constructor(ctx: Context) : SQLiteOpenHelper(ctx, "smsmanager.db", null, 2) {

    override fun onCreate(db: SQLiteDatabase) {
        db.execSQL(
            """CREATE TABLE rules (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                action TEXT NOT NULL, target TEXT NOT NULL, pattern TEXT NOT NULL,
                mode TEXT NOT NULL DEFAULT 'WILDCARD', enabled INTEGER NOT NULL DEFAULT 1,
                hits INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL)"""
        )
        db.execSQL(
            """CREATE TABLE blocked (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                sender TEXT NOT NULL, body TEXT NOT NULL, date INTEGER NOT NULL,
                rule_id INTEGER NOT NULL, rule_label TEXT NOT NULL,
                blocked_at INTEGER NOT NULL DEFAULT 0)"""
        )
    }

    override fun onUpgrade(db: SQLiteDatabase, oldVersion: Int, newVersion: Int) {
        if (oldVersion < 2) {
            // v1 stored a regex flag; the old column stays (unused, defaults to 0) because SQLite can't cheaply drop it.
            db.execSQL("ALTER TABLE rules ADD COLUMN mode TEXT NOT NULL DEFAULT 'WILDCARD'")
            db.execSQL("UPDATE rules SET mode = CASE regex WHEN 1 THEN 'REGEX' ELSE 'WILDCARD' END")
            // Retention counts from when a message was blocked, not from when it was sent.
            db.execSQL("ALTER TABLE blocked ADD COLUMN blocked_at INTEGER NOT NULL DEFAULT 0")
            db.execSQL("UPDATE blocked SET blocked_at = date")
        }
    }

    // ---- rules ----

    fun rules(): List<Rule> = readableDatabase.query("rules", null, null, null, null, null, "id DESC").use { c ->
        buildList { while (c.moveToNext()) add(c.toRule()) }
    }

    /** Inserts when [rule].id is 0, otherwise updates the row. Returns the row id. */
    fun saveRule(rule: Rule): Long {
        val v = ContentValues().apply {
            put("action", rule.action.name)
            put("target", rule.target.name)
            put("pattern", rule.pattern)
            put("mode", rule.mode.name)
            put("enabled", if (rule.enabled) 1 else 0)
        }
        val id = if (rule.id == 0L) {
            v.put("hits", 0)
            v.put("created_at", rule.createdAt)
            writableDatabase.insertOrThrow("rules", null, v)
        } else {
            writableDatabase.update("rules", v, "id=?", arrayOf(rule.id.toString()))
            rule.id
        }
        Ticker.bump()
        return id
    }

    fun deleteRule(id: Long) {
        writableDatabase.delete("rules", "id=?", arrayOf(id.toString()))
        Ticker.bump()
    }

    fun setRuleEnabled(id: Long, enabled: Boolean) {
        val v = ContentValues().apply { put("enabled", if (enabled) 1 else 0) }
        writableDatabase.update("rules", v, "id=?", arrayOf(id.toString()))
        Ticker.bump()
    }

    fun bumpHit(id: Long) {
        writableDatabase.execSQL("UPDATE rules SET hits = hits + 1 WHERE id = ?", arrayOf(id))
    }

    /** True when an equivalent block rule already exists, so quick "block this number" taps don't pile up. */
    fun hasRule(action: RuleAction, target: RuleTarget, pattern: String): Boolean =
        readableDatabase.query(
            "rules", arrayOf("id"), "action=? AND target=? AND pattern=? AND mode='WILDCARD'",
            arrayOf(action.name, target.name, pattern), null, null, null,
        ).use { it.moveToFirst() }

    // ---- blocked ----

    fun blocked(): List<BlockedMessage> =
        readableDatabase.query("blocked", null, null, null, null, null, "date DESC").use { c ->
            buildList {
                while (c.moveToNext()) add(
                    BlockedMessage(
                        id = c.getLong(c.getColumnIndexOrThrow("id")),
                        sender = c.getString(c.getColumnIndexOrThrow("sender")),
                        body = c.getString(c.getColumnIndexOrThrow("body")),
                        date = c.getLong(c.getColumnIndexOrThrow("date")),
                        ruleId = c.getLong(c.getColumnIndexOrThrow("rule_id")),
                        ruleLabel = c.getString(c.getColumnIndexOrThrow("rule_label")),
                    )
                )
            }
        }

    fun blockedCount(): Int =
        readableDatabase.rawQuery("SELECT COUNT(*) FROM blocked", null).use { if (it.moveToFirst()) it.getInt(0) else 0 }

    /** [date] is when the message was sent; retention is counted from now. Pass notify=false when adding in bulk. */
    fun addBlocked(sender: String, body: String, date: Long, rule: Rule, notify: Boolean = true): Long {
        val v = ContentValues().apply {
            put("sender", sender)
            put("body", body)
            put("date", date)
            put("rule_id", rule.id)
            put("rule_label", rule.label)
            put("blocked_at", System.currentTimeMillis())
        }
        val id = writableDatabase.insertOrThrow("blocked", null, v)
        if (notify) Ticker.bump()
        return id
    }

    fun deleteBlockedIds(ids: List<Long>) {
        if (ids.isEmpty()) return
        writableDatabase.delete("blocked", "id IN (${ids.joinToString(",")})", null)
        Ticker.bump()
    }

    fun addHits(ruleId: Long, n: Int) {
        writableDatabase.execSQL("UPDATE rules SET hits = hits + ? WHERE id = ?", arrayOf(n, ruleId))
    }

    /** Adds rules that aren't already present (same action, target, pattern and mode). Returns how many were added. */
    fun importRules(incoming: List<Rule>): Int {
        val have = rules().mapTo(HashSet()) { listOf(it.action, it.target, it.pattern, it.mode) }
        var added = 0
        for (r in incoming) {
            if (!have.add(listOf(r.action, r.target, r.pattern, r.mode))) continue
            val v = ContentValues().apply {
                put("action", r.action.name)
                put("target", r.target.name)
                put("pattern", r.pattern)
                put("mode", r.mode.name)
                put("enabled", if (r.enabled) 1 else 0)
                put("hits", r.hits)
                put("created_at", r.createdAt)
            }
            writableDatabase.insertOrThrow("rules", null, v)
            added++
        }
        if (added > 0) Ticker.bump()
        return added
    }

    fun deleteBlocked(id: Long) {
        writableDatabase.delete("blocked", "id=?", arrayOf(id.toString()))
        Ticker.bump()
    }

    fun clearBlocked() {
        writableDatabase.delete("blocked", null, null)
        Ticker.bump()
    }

    fun purgeBlocked(olderThan: Long) {
        if (writableDatabase.delete("blocked", "blocked_at < ?", arrayOf(olderThan.toString())) > 0) Ticker.bump()
    }

    private fun Cursor.toRule() = Rule(
        id = getLong(getColumnIndexOrThrow("id")),
        action = RuleAction.valueOf(getString(getColumnIndexOrThrow("action"))),
        target = RuleTarget.valueOf(getString(getColumnIndexOrThrow("target"))),
        pattern = getString(getColumnIndexOrThrow("pattern")),
        mode = MatchMode.valueOf(getString(getColumnIndexOrThrow("mode"))),
        enabled = getInt(getColumnIndexOrThrow("enabled")) != 0,
        hits = getInt(getColumnIndexOrThrow("hits")),
        createdAt = getLong(getColumnIndexOrThrow("created_at")),
    )

    companion object {
        @Volatile private var instance: Db? = null
        fun get(ctx: Context): Db = instance ?: synchronized(this) {
            instance ?: Db(ctx.applicationContext).also { instance = it }
        }
    }
}

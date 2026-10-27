package com.omar.smsmanager.data

import android.content.Context

class Settings(ctx: Context) {
    private val p = ctx.applicationContext.getSharedPreferences("settings", Context.MODE_PRIVATE)

    /** Senders found in the address book are never blocked. */
    var contactsAlwaysAllowed: Boolean
        get() = p.getBoolean("contacts_allowed", true)
        set(v) { p.edit().putBoolean("contacts_allowed", v).apply(); Ticker.bump() }

    /** Days to keep blocked messages. 0 = forever. */
    var retentionDays: Int
        get() = p.getInt("retention_days", 30)
        set(v) { p.edit().putInt("retention_days", v).apply(); Ticker.bump() }

    // Automatic backup file. The URI permission dies with an uninstall, so after a reinstall it is chosen again.
    var backupUri: String
        get() = p.getString("backup_uri", "").orEmpty()
        set(v) = p.edit().putString("backup_uri", v).apply()

    var backupLast: Long
        get() = p.getLong("backup_last", 0)
        set(v) = p.edit().putLong("backup_last", v).apply()

    /** Hash of the content last written, so an unchanged state doesn't rewrite the file. */
    var backupHash: Int
        get() = p.getInt("backup_hash", 0)
        set(v) = p.edit().putInt("backup_hash", v).apply()

    var backupError: String
        get() = p.getString("backup_error", "").orEmpty()
        set(v) = p.edit().putString("backup_error", v).apply()

    companion object {
        const val DAY_MS = 24L * 60 * 60 * 1000
    }
}

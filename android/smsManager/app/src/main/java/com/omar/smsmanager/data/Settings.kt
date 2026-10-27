package com.omar.smsmanager.data

import android.content.Context

class Settings(ctx: Context) {
    private val p = ctx.applicationContext.getSharedPreferences("settings", Context.MODE_PRIVATE)

    /** Senders found in the address book are never blocked. */
    var contactsAlwaysAllowed: Boolean
        get() = p.getBoolean("contacts_allowed", true)
        set(v) { p.edit().putBoolean("contacts_allowed", v).apply(); Ticker.bump() }

    /** Adds a "Copy 482913" button to the notification when a message contains a one-time code. */
    var offerOtpCopy: Boolean
        get() = p.getBoolean("offer_otp_copy", true)
        set(v) { p.edit().putBoolean("offer_otp_copy", v).apply(); Ticker.bump() }

    /** Days to keep blocked messages. 0 = forever. */
    var retentionDays: Int
        get() = p.getInt("retention_days", 30)
        set(v) { p.edit().putInt("retention_days", v).apply(); Ticker.bump() }

    // ---- forwarding to Telegram ----

    var forwardEnabled: Boolean
        get() = p.getBoolean("fwd_enabled", false)
        set(v) { p.edit().putBoolean("fwd_enabled", v).apply(); Ticker.bump() }

    /** The Telegram chat messages go to (a number as text). Empty until connected. */
    var forwardChatId: String
        get() = p.getString("fwd_chat", "").orEmpty()
        set(v) { p.edit().putString("fwd_chat", v).apply(); Ticker.bump() }

    /** Shown in Settings so you can see which account is connected. */
    var forwardChatName: String
        get() = p.getString("fwd_chat_name", "").orEmpty()
        set(v) { p.edit().putString("fwd_chat_name", v).apply(); Ticker.bump() }

    /** The bot token, encrypted with a Keystore key. Not part of the backup file. */
    var forwardTokenEnc: String
        get() = p.getString("fwd_token", "").orEmpty()
        set(v) { p.edit().putString("fwd_token", v).apply(); Ticker.bump() }

    /** Sender patterns, one per line. Empty means everyone. */
    var forwardOnlySenders: String
        get() = p.getString("fwd_senders", "").orEmpty()
        set(v) { p.edit().putString("fwd_senders", v).apply(); Ticker.bump() }

    var forwardSkipCodes: Boolean
        get() = p.getBoolean("fwd_skip_codes", false)
        set(v) { p.edit().putBoolean("fwd_skip_codes", v).apply(); Ticker.bump() }

    var forwardLastSent: Long
        get() = p.getLong("fwd_last_sent", 0)
        set(v) = p.edit().putLong("fwd_last_sent", v).apply()

    var forwardError: String
        get() = p.getString("fwd_error", "").orEmpty()
        set(v) = p.edit().putString("fwd_error", v).apply()

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

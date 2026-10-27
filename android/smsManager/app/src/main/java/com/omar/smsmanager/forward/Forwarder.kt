package com.omar.smsmanager.forward

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.job.JobInfo
import android.app.job.JobScheduler
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.os.Build
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import com.omar.smsmanager.MainActivity
import com.omar.smsmanager.R
import com.omar.smsmanager.data.Contacts
import com.omar.smsmanager.data.Db
import com.omar.smsmanager.data.OutboxItem
import com.omar.smsmanager.data.Settings
import com.omar.smsmanager.data.Ticker
import java.io.IOException
import java.net.UnknownHostException
import java.util.concurrent.atomic.AtomicBoolean
import javax.net.ssl.SSLException

/**
 * Sends each incoming message to the user's own Telegram chat through their private bot.
 * Messages are queued in the database first, so nothing is lost while the phone is offline; a job that waits
 * for a network connection empties the queue.
 */
object Forwarder {
    private const val JOB_ID = 4711
    private const val CHANNEL = "forwarding"
    private const val NOTIFICATION_ID = 4711
    private const val GIVE_UP_AFTER_MS = 3 * 24 * 60 * 60 * 1000L

    private val dirty = AtomicBoolean(false)
    private val draining = Any()

    /** Switched on, connected to a chat, and the token can still be decrypted. */
    fun isReady(ctx: Context): Boolean {
        val s = Settings(ctx)
        return s.forwardEnabled && s.forwardChatId.toLongOrNull() != null && token(ctx) != null
    }

    fun token(ctx: Context): String? = Secret.decrypt(Settings(ctx).forwardTokenEnc)

    fun onReceived(ctx: Context, sender: String, body: String, sentAt: Long) {
        try {
            if (!isReady(ctx)) return
            val s = Settings(ctx)
            if (!ForwardPolicy.shouldForward(ForwardPolicy.parseSenders(s.forwardOnlySenders), s.forwardSkipCodes, sender, body)) return
            Db.get(ctx).enqueueOutbox(sender, body, sentAt)
            kick(ctx)
        } catch (_: Exception) {
            // Forwarding must never get in the way of receiving the message.
        }
    }

    /** Asks the system to run the queue as soon as there is a network connection. */
    fun kick(ctx: Context) {
        dirty.set(true)
        val js = ctx.getSystemService(JobScheduler::class.java)
        if (js.getPendingJob(JOB_ID) != null) return
        val b = JobInfo.Builder(JOB_ID, ComponentName(ctx, ForwardJobService::class.java))
            .setRequiredNetworkType(JobInfo.NETWORK_TYPE_ANY)
            .setBackoffCriteria(30_000, JobInfo.BACKOFF_POLICY_EXPONENTIAL)
            .setPersisted(true)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) b.setExpedited(true)
        js.schedule(b.build())
    }

    /** Sends everything in the queue. Returns true when the job should be retried later. Runs on a worker thread. */
    fun drain(ctx: Context): Boolean = synchronized(draining) {
        val db = Db.get(ctx)
        val s = Settings(ctx)
        db.purgeOutbox(System.currentTimeMillis() - GIVE_UP_AFTER_MS)
        do {
            dirty.set(false)
            val items = db.outbox()
            if (items.isEmpty()) continue
            val chat = s.forwardChatId.toLongOrNull()
            val tok = token(ctx)
            if (!s.forwardEnabled || chat == null || tok == null) return false // kept until it is fixed
            val client = TelegramClient(tok)
            try {
                for (item in items) {
                    try {
                        client.sendMessage(chat, compose(ctx, item))
                    } catch (e: TelegramException) {
                        if (!e.refused) throw e
                        // Telegram refuses this one for good; don't block the rest behind it.
                        s.forwardError = describe(e)
                    }
                    db.deleteOutbox(item.id)
                    s.forwardLastSent = System.currentTimeMillis()
                    Ticker.bump()
                }
                if (s.forwardError.isNotEmpty()) { s.forwardError = ""; Ticker.bump() }
            } catch (e: TelegramException) {
                fail(ctx, describe(e))
                // A bad token or a blocked bot: retrying only repeats it. The queue is kept and sent after the user fixes it.
                return !e.needsFix
            } catch (e: IOException) {
                fail(ctx, describe(e))
                return true
            }
        } while (dirty.get() || db.outboxCount() > 0)
        false
    }

    private fun compose(ctx: Context, item: OutboxItem): String {
        val name = Contacts.name(ctx, item.sender)
        val from = if (name != null) "$name (${item.sender})" else item.sender
        return "\uD83D\uDCE9 SMS from $from\n\n${item.body}"
    }

    sealed class Connect {
        class Ok(val chat: TelegramChat) : Connect()
        class Failed(val reason: String) : Connect()
    }

    /**
     * Finds the one person who has messaged the bot and saves the token and that chat. Refuses if nobody has,
     * or if more than one chat has: then the bot isn't private any more.
     */
    fun connect(ctx: Context, token: String): Connect {
        if (!TelegramClient.looksLikeToken(token)) return Connect.Failed("That doesn't look like a bot token. It looks like 123456789:AAH...")
        val client = TelegramClient(token)
        val chats = try {
            client.findChats()
        } catch (e: Exception) {
            return Connect.Failed(describe(e))
        }
        when {
            chats.isEmpty() -> return Connect.Failed("Nobody has messaged this bot yet. Open it in Telegram, send it any message, then try again.")
            chats.size > 1 -> return Connect.Failed(
                "${chats.size} different people have messaged this bot (${chats.joinToString { it.name }}), so it isn't private. " +
                    "Make a new bot with @BotFather and don't share its name."
            )
        }
        val chat = chats.single()
        try {
            client.sendMessage(chat.id, "SmsManager is connected. Incoming text messages will appear here, labelled \"SMS from ...\".")
        } catch (e: Exception) {
            return Connect.Failed(describe(e))
        }
        val s = Settings(ctx)
        s.forwardTokenEnc = Secret.encrypt(token)
        s.forwardChatId = chat.id.toString()
        s.forwardChatName = chat.name
        s.forwardError = ""
        return Connect.Ok(chat)
    }

    /** Sends one message right now. Returns an explanation if it failed, null if it worked. */
    fun sendTest(ctx: Context): String? {
        val tok = token(ctx)
        val chat = Settings(ctx).forwardChatId.toLongOrNull()
        if (tok == null || chat == null) return "Not connected yet."
        return try {
            TelegramClient(tok).sendMessage(chat, "Test message from SmsManager. Forwarding works.")
            null
        } catch (e: Exception) {
            describe(e)
        }
    }

    fun describe(e: Exception): String = when {
        e is TelegramException && (e.code == 401 || e.code == 404) -> "Telegram doesn't recognise this bot token. Check it, or make a new token with /revoke in @BotFather."
        e is TelegramException && e.code == 403 -> "The bot can't write to you. Open it in Telegram and tap Start (or unblock it)."
        e is TelegramException && e.code == 409 -> "Telegram says this bot is in use by something else (a webhook). Make a new bot."
        e is TelegramException && e.code == 429 -> "Telegram asked us to slow down. Will retry."
        e is TelegramException -> "Telegram refused the message: ${e.message.orEmpty()}"
        e is UnknownHostException -> "No network connection."
        e is SSLException -> "Secure connection failed: ${e.message.orEmpty()}"
        else -> "Couldn't reach Telegram: ${e.message ?: e.javaClass.simpleName}"
    }

    private fun fail(ctx: Context, message: String) {
        val s = Settings(ctx)
        val first = s.forwardError != message
        s.forwardError = message
        Ticker.bump()
        if (first) notifyFailure(ctx, message)
    }

    private fun notifyFailure(ctx: Context, message: String) {
        val nm = NotificationManagerCompat.from(ctx)
        if (!nm.areNotificationsEnabled()) return
        ctx.getSystemService(NotificationManager::class.java)
            .createNotificationChannel(NotificationChannel(CHANNEL, "Forwarding problems", NotificationManager.IMPORTANCE_DEFAULT))
        val open = PendingIntent.getActivity(
            ctx, NOTIFICATION_ID, Intent(ctx, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val n = NotificationCompat.Builder(ctx, CHANNEL)
            .setSmallIcon(R.drawable.ic_stat_message)
            .setContentTitle("Couldn't forward messages to Telegram")
            .setContentText(message)
            .setStyle(NotificationCompat.BigTextStyle().bigText(message))
            .setContentIntent(open)
            .setAutoCancel(true)
            .build()
        try { nm.notify(NOTIFICATION_ID, n) } catch (_: SecurityException) {}
    }
}

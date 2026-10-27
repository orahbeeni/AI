package com.omar.smsmanager.sms

import android.annotation.SuppressLint
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.app.RemoteInput
import com.omar.smsmanager.MainActivity
import com.omar.smsmanager.R
import com.omar.smsmanager.data.Contacts
import com.omar.smsmanager.data.Settings

object Notifier {
    private const val CHANNEL = "messages"
    const val KEY_REPLY = "reply"

    fun ensureChannel(ctx: Context) {
        val nm = ctx.getSystemService(NotificationManager::class.java)
        if (nm.getNotificationChannel(CHANNEL) == null) {
            nm.createNotificationChannel(NotificationChannel(CHANNEL, "Messages", NotificationManager.IMPORTANCE_HIGH))
        }
    }

    fun idFor(threadId: Long, address: String): Int = if (threadId > 0) threadId.toInt() else address.hashCode()

    @SuppressLint("MissingPermission")
    fun showMessage(ctx: Context, address: String, body: String, threadId: Long) {
        ensureChannel(ctx)
        val nm = NotificationManagerCompat.from(ctx)
        if (!nm.areNotificationsEnabled()) return

        val id = idFor(threadId, address)
        val title = Contacts.name(ctx, address) ?: address

        val open = Intent(ctx, MainActivity::class.java)
            .setAction(MainActivity.ACTION_OPEN_THREAD)
            .putExtra(MainActivity.EXTRA_THREAD, threadId)
            .putExtra(MainActivity.EXTRA_ADDRESS, address)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP)
        val openPi = PendingIntent.getActivity(ctx, id, open, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)

        val actionIntent = { action: String ->
            Intent(ctx, NotificationActionReceiver::class.java)
                .setAction(action)
                .putExtra(NotificationActionReceiver.EXTRA_THREAD, threadId)
                .putExtra(NotificationActionReceiver.EXTRA_ADDRESS, address)
        }
        // RemoteInput needs a mutable PendingIntent so the system can attach the typed reply.
        val replyPi = PendingIntent.getBroadcast(
            ctx, id, actionIntent(NotificationActionReceiver.ACTION_REPLY),
            PendingIntent.FLAG_MUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val readPi = PendingIntent.getBroadcast(
            ctx, id, actionIntent(NotificationActionReceiver.ACTION_MARK_READ),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )

        val reply = NotificationCompat.Action.Builder(
            R.drawable.ic_stat_message, "Reply", replyPi,
        ).addRemoteInput(RemoteInput.Builder(KEY_REPLY).setLabel("Reply").build()).build()
        val markRead = NotificationCompat.Action.Builder(R.drawable.ic_stat_message, "Mark as read", readPi).build()

        val builder = NotificationCompat.Builder(ctx, CHANNEL)
            .setSmallIcon(R.drawable.ic_stat_message)
            .setContentTitle(title)
            .setContentText(body)
            .setStyle(NotificationCompat.BigTextStyle().bigText(body))
            .setCategory(NotificationCompat.CATEGORY_MESSAGE)
            .setPriority(NotificationCompat.PRIORITY_HIGH) // heads-up banner
            .setContentIntent(openPi)
            .setAutoCancel(true)

        // A one-time code gets a button with the code on it, first in line, so one tap on the banner copies it.
        val code = if (Settings(ctx).offerOtpCopy) OtpExtractor.extract(body) else null
        if (code != null) {
            val copyPi = PendingIntent.getBroadcast(
                ctx, id, actionIntent(NotificationActionReceiver.ACTION_COPY).putExtra(NotificationActionReceiver.EXTRA_CODE, code),
                PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
            )
            builder.addAction(NotificationCompat.Action.Builder(R.drawable.ic_stat_message, "Copy $code", copyPi).build())
        }

        nm.notify(id, builder.addAction(reply).addAction(markRead).build())
    }

    @SuppressLint("MissingPermission")
    fun showMmsUnsupported(ctx: Context) {
        ensureChannel(ctx)
        val nm = NotificationManagerCompat.from(ctx)
        if (!nm.areNotificationsEnabled()) return
        val n = NotificationCompat.Builder(ctx, CHANNEL)
            .setSmallIcon(R.drawable.ic_stat_message)
            .setContentTitle("Picture/group message not received")
            .setContentText("SmsManager can't download MMS yet. Switch default SMS app to read it.")
            .setAutoCancel(true)
            .build()
        nm.notify(MMS_NOTIFICATION_ID, n)
    }

    fun cancel(ctx: Context, threadId: Long, address: String) {
        NotificationManagerCompat.from(ctx).cancel(idFor(threadId, address))
    }

    private const val MMS_NOTIFICATION_ID = 0x4d4d53
}

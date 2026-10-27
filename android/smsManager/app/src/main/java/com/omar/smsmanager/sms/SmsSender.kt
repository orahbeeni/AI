package com.omar.smsmanager.sms

import android.app.PendingIntent
import android.content.ContentValues
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.provider.Telephony.Sms
import android.telephony.SmsManager

object SmsSender {
    const val ACTION_SENT = "com.omar.smsmanager.SMS_SENT"

    private fun manager(ctx: Context): SmsManager =
        if (Build.VERSION.SDK_INT >= 31) ctx.getSystemService(SmsManager::class.java)
        else @Suppress("DEPRECATION") SmsManager.getDefault()

    /** Sends [body] and records it in the system conversation so it shows up in the thread. */
    fun send(ctx: Context, address: String, body: String): Boolean {
        val app = ctx.applicationContext
        val text = body.trim()
        val to = address.trim()
        if (text.isEmpty() || to.isEmpty()) return false

        val uri: Uri? = try {
            val v = ContentValues().apply {
                put(Sms.ADDRESS, to)
                put(Sms.BODY, text)
                put(Sms.DATE, System.currentTimeMillis())
                put(Sms.READ, 1)
                put(Sms.SEEN, 1)
            }
            app.contentResolver.insert(Sms.Outbox.CONTENT_URI, v)
        } catch (_: Exception) {
            null
        }

        return try {
            val mgr = manager(app)
            val parts = mgr.divideMessage(text)
            val sentIntents = uri?.let { u ->
                ArrayList<PendingIntent>(parts.size).also { list ->
                    repeat(parts.size) {
                        val i = Intent(app, SentStatusReceiver::class.java).setAction(ACTION_SENT).setData(u)
                        list.add(PendingIntent.getBroadcast(app, 0, i, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT))
                    }
                }
            }
            mgr.sendMultipartTextMessage(to, null, parts, sentIntents, null)
            true
        } catch (_: Exception) {
            uri?.let { SentStatusReceiver.markFailed(app, it) }
            false
        }
    }
}

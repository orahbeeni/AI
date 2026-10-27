package com.omar.smsmanager.sms

import android.app.Activity
import android.content.BroadcastReceiver
import android.content.ContentValues
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.provider.Telephony.Sms

/** Flips an outgoing message from "sending" to "sent" or "failed" once the radio reports back. */
class SentStatusReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val uri = intent.data ?: return
        try {
            if (resultCode == Activity.RESULT_OK) {
                // Only promote a message still in the outbox, so one failed part keeps the whole message failed.
                val v = ContentValues().apply { put(Sms.TYPE, Sms.MESSAGE_TYPE_SENT) }
                context.contentResolver.update(uri, v, "${Sms.TYPE}=?", arrayOf(Sms.MESSAGE_TYPE_OUTBOX.toString()))
            } else {
                markFailed(context, uri)
            }
        } catch (_: Exception) {
        }
    }

    companion object {
        fun markFailed(ctx: Context, uri: Uri) {
            try {
                val v = ContentValues().apply { put(Sms.TYPE, Sms.MESSAGE_TYPE_FAILED) }
                ctx.contentResolver.update(uri, v, null, null)
            } catch (_: Exception) {
            }
        }
    }
}

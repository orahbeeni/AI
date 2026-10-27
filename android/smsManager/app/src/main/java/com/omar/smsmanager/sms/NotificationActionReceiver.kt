package com.omar.smsmanager.sms

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import androidx.core.app.RemoteInput
import com.omar.smsmanager.data.SmsStore
import kotlin.concurrent.thread

/** Handles the "Reply" and "Mark as read" buttons on message notifications. */
class NotificationActionReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val threadId = intent.getLongExtra(EXTRA_THREAD, 0)
        val address = intent.getStringExtra(EXTRA_ADDRESS) ?: return
        val reply = RemoteInput.getResultsFromIntent(intent)?.getCharSequence(Notifier.KEY_REPLY)?.toString()

        val pending = goAsync()
        thread {
            try {
                if (intent.action == ACTION_REPLY && !reply.isNullOrBlank()) SmsSender.send(context, address, reply)
                if (threadId > 0) SmsStore.markRead(context, threadId)
                Notifier.cancel(context, threadId, address)
            } finally {
                pending.finish()
            }
        }
    }

    companion object {
        const val ACTION_REPLY = "com.omar.smsmanager.REPLY"
        const val ACTION_MARK_READ = "com.omar.smsmanager.MARK_READ"
        const val EXTRA_THREAD = "thread"
        const val EXTRA_ADDRESS = "address"
    }
}

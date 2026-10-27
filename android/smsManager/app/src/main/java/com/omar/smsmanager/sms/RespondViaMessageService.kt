package com.omar.smsmanager.sms

import android.app.Service
import android.content.Intent
import android.net.Uri
import android.os.IBinder
import kotlin.concurrent.thread

/** "Reply with message" from the incoming-call screen. */
class RespondViaMessageService : Service() {
    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val to = intent?.data?.schemeSpecificPart?.let { Uri.decode(it).substringBefore('?') }
        val text = intent?.getStringExtra(Intent.EXTRA_TEXT)
        if (to.isNullOrBlank() || text.isNullOrBlank()) {
            stopSelf(startId)
            return START_NOT_STICKY
        }
        thread {
            to.split(';', ',').map { it.trim() }.filter { it.isNotEmpty() }.forEach { SmsSender.send(this, it, text) }
            stopSelf(startId)
        }
        return START_NOT_STICKY
    }
}

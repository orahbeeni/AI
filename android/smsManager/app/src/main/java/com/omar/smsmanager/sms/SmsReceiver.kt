package com.omar.smsmanager.sms

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.provider.Telephony
import com.omar.smsmanager.data.Contacts
import com.omar.smsmanager.data.Db
import com.omar.smsmanager.data.Settings
import com.omar.smsmanager.data.SmsStore
import com.omar.smsmanager.spam.SpamFilter
import com.omar.smsmanager.spam.Verdict
import kotlin.concurrent.thread

/**
 * Only the default SMS app gets SMS_DELIVER, and nothing reaches the inbox unless we put it there.
 * That is what makes blocking possible: a blocked message is simply never written to the provider.
 */
class SmsReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != Telephony.Sms.Intents.SMS_DELIVER_ACTION) return
        val pdus = Telephony.Sms.Intents.getMessagesFromIntent(intent)
        if (pdus.isNullOrEmpty()) return
        val sender = pdus[0].displayOriginatingAddress ?: pdus[0].originatingAddress ?: return
        val body = pdus.joinToString("") { it.displayMessageBody.orEmpty() }
        val sentAt = pdus[0].timestampMillis
        val subscription = intent.getIntExtra("subscription", -1)

        val pending = goAsync()
        thread {
            try {
                Inbound.handle(context.applicationContext, sender, body, sentAt, subscription)
            } finally {
                pending.finish()
            }
        }
    }
}

object Inbound {
    fun handle(ctx: Context, sender: String, body: String, sentAt: Long, subscription: Int) {
        val settings = Settings(ctx)

        val db = Db.get(ctx)
        val isContact = settings.contactsAlwaysAllowed && Contacts.isContact(ctx, sender)
        when (val verdict = SpamFilter(db.rules()).evaluate(sender, body, isContact)) {
            is Verdict.Blocked -> {
                db.bumpHit(verdict.rule.id)
                db.addBlocked(sender, body, System.currentTimeMillis(), verdict.rule)
            }
            else -> {
                val uri = SmsStore.insertInbox(ctx, sender, body, sentAt, subscription, read = false) ?: return
                if (verdict is Verdict.Allowed && verdict.rule != null) db.bumpHit(verdict.rule.id)
                Notifier.showMessage(ctx, sender, body, SmsStore.threadIdOf(ctx, uri))
            }
        }
    }
}

package com.omar.smsmanager.sms

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent

/**
 * Required for default-SMS eligibility. MMS download isn't implemented yet, so all we can do is
 * tell the user that something arrived which they won't see here.
 */
class MmsReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        Notifier.showMmsUnsupported(context)
    }
}

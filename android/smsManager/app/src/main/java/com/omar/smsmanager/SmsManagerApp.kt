package com.omar.smsmanager

import android.app.Application
import com.omar.smsmanager.data.AutoBackup
import com.omar.smsmanager.data.Db
import com.omar.smsmanager.data.Ticker
import com.omar.smsmanager.forward.Forwarder
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.launch

class SmsManagerApp : Application() {
    override fun onCreate() {
        super.onCreate()
        // Pick up anything that was still waiting when the app last stopped.
        CoroutineScope(SupervisorJob() + Dispatchers.IO).launch {
            try {
                if (Forwarder.isReady(this@SmsManagerApp) && Db.get(this@SmsManagerApp).outboxCount() > 0) Forwarder.kick(this@SmsManagerApp)
            } catch (_: Exception) {}
        }
        // Any change to rules or settings bumps the ticker; wait for it to go quiet, then save the backup file.
        CoroutineScope(SupervisorJob() + Dispatchers.IO).launch {
            Ticker.tick.collectLatest {
                delay(3000)
                AutoBackup.writeIfChanged(this@SmsManagerApp)
            }
        }
    }
}

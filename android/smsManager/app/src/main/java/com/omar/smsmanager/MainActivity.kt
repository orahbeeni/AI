package com.omar.smsmanager

import android.content.Intent
import android.database.ContentObserver
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.provider.Telephony
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.runtime.mutableStateOf
import com.omar.smsmanager.data.Db
import com.omar.smsmanager.data.Settings
import com.omar.smsmanager.data.Ticker
import com.omar.smsmanager.sms.Notifier
import com.omar.smsmanager.ui.App
import com.omar.smsmanager.ui.AppTheme
import com.omar.smsmanager.ui.Route
import kotlin.concurrent.thread

class MainActivity : ComponentActivity() {
    private val pendingRoute = mutableStateOf<Route?>(null)

    private val smsObserver = object : ContentObserver(Handler(Looper.getMainLooper())) {
        override fun onChange(selfChange: Boolean) = Ticker.bump()
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        Notifier.ensureChannel(this)
        if (savedInstanceState == null) pendingRoute.value = routeFrom(intent)

        thread {
            val days = Settings(this).retentionDays
            if (days > 0) Db.get(this).purgeBlocked(System.currentTimeMillis() - days * Settings.DAY_MS)
        }

        setContent { AppTheme { App(pendingRoute) } }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        pendingRoute.value = routeFrom(intent)
    }

    override fun onStart() {
        super.onStart()
        contentResolver.registerContentObserver(Telephony.Sms.CONTENT_URI, true, smsObserver)
    }

    override fun onStop() {
        contentResolver.unregisterContentObserver(smsObserver)
        super.onStop()
    }

    override fun onResume() {
        super.onResume()
        Ticker.bump() // default-app status and permissions may have changed while we were away
    }

    private fun routeFrom(intent: Intent): Route? {
        when (intent.action) {
            ACTION_OPEN_THREAD -> {
                val address = intent.getStringExtra(EXTRA_ADDRESS) ?: return null
                return Route.Thread(intent.getLongExtra(EXTRA_THREAD, 0), address)
            }
            Intent.ACTION_SEND, Intent.ACTION_SENDTO -> {
                val to = intent.data?.schemeSpecificPart?.let { android.net.Uri.decode(it).substringBefore('?') }.orEmpty()
                val body = intent.getStringExtra("sms_body") ?: intent.getStringExtra(Intent.EXTRA_TEXT).orEmpty()
                return Route.Compose(to, body)
            }
        }
        return null
    }

    companion object {
        const val ACTION_OPEN_THREAD = "com.omar.smsmanager.OPEN_THREAD"
        const val EXTRA_THREAD = "thread"
        const val EXTRA_ADDRESS = "address"
    }
}

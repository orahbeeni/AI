package com.omar.wifishare

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.IBinder
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.launch

/**
 * Keeps the app alive while it is not on screen, so the computer can still browse this phone
 * and copies finish. Its notification says which computer it is linked to.
 */
class LinkService : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
    private var watching: Job? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == STOP) {
            Engine.background = false
            stopSelf()
            return START_NOT_STICKY
        }
        ensureChannel(this)
        ServiceCompat.startForeground(
            this,
            NOTIFICATION,
            build(Engine.state.value),
            ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE,
        )
        if (watching == null) {
            watching = scope.launch {
                Engine.state.map { summary(it) }.distinctUntilChanged().collect {
                    getSystemService(NotificationManager::class.java).notify(NOTIFICATION, build(Engine.state.value))
                    // Started only to finish copies: done once they are.
                    if (!Engine.background && Engine.state.value.copies.all { it.finished }) stopSelf()
                }
            }
        }
        return START_STICKY
    }

    override fun onDestroy() {
        scope.cancel()
        super.onDestroy()
    }

    private fun build(st: EngineState): Notification {
        val (title, text) = summary(st)
        val open = PendingIntent.getActivity(
            this, 0, Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE,
        )
        val stop = PendingIntent.getService(
            this, 1, Intent(this, LinkService::class.java).setAction(STOP), PendingIntent.FLAG_IMMUTABLE,
        )
        return NotificationCompat.Builder(this, CHANNEL)
            .setSmallIcon(R.drawable.ic_stat_link)
            .setContentTitle(title)
            .setContentText(text)
            .setContentIntent(open)
            .setOngoing(true)
            .setSilent(true)
            .addAction(0, "Stop", stop)
            .build()
    }

    companion object {
        private const val CHANNEL = "link"
        private const val NOTIFICATION = 1
        private const val STOP = "stop"

        /** Runs the service if the user wants the link kept in the background. */
        fun sync(context: Context) {
            val want = Engine.background && Engine.state.value.computers.isNotEmpty()
            val intent = Intent(context, LinkService::class.java)
            if (want) ContextCompat.startForegroundService(context, intent) else context.stopService(intent)
        }

        /** Keeps the app running until running copies finish, whatever the setting. */
        fun keepForCopies(context: Context) {
            ContextCompat.startForegroundService(context, Intent(context, LinkService::class.java))
        }

        private fun ensureChannel(context: Context) {
            val nm = context.getSystemService(NotificationManager::class.java)
            if (nm.getNotificationChannel(CHANNEL) == null) {
                nm.createNotificationChannel(
                    NotificationChannel(CHANNEL, "Link to your computer", NotificationManager.IMPORTANCE_LOW),
                )
            }
        }

        /** The notification's title and text. */
        fun summary(st: EngineState): Pair<String, String> {
            val running = st.copies.filter { !it.finished }
            val link = st.link
            val title = when (link.state) {
                "connected" -> "Linked to ${link.computer}"
                "searching" -> "Looking for ${link.computer}…"
                else -> "Not linked to a computer"
            }
            val text = if (running.isNotEmpty()) {
                val done = running.sumOf { it.done }
                val total = running.sumOf { it.total }.coerceAtLeast(1)
                "Copying ${running.size} · ${(done * 100 / total)}%"
            } else if (link.connected) {
                "It can browse this phone"
            } else {
                "Tap to open wifishare"
            }
            return title to text
        }
    }
}

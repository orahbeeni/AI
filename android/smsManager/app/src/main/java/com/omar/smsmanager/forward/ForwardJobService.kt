package com.omar.smsmanager.forward

import android.app.job.JobParameters
import android.app.job.JobService
import kotlin.concurrent.thread

class ForwardJobService : JobService() {
    override fun onStartJob(params: JobParameters): Boolean {
        thread {
            val retry = try { Forwarder.drain(applicationContext) } catch (_: Exception) { true }
            jobFinished(params, retry)
        }
        return true
    }

    // Dropped by the system (lost network, say): run again when conditions allow. Nothing is lost, items are removed only once sent.
    override fun onStopJob(params: JobParameters) = true
}

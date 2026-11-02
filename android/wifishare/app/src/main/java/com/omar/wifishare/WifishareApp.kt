package com.omar.wifishare

import android.app.Application

class WifishareApp : Application() {
    override fun onCreate() {
        super.onCreate()
        Engine.init(this)
        Outgoing.prune(this)
    }
}

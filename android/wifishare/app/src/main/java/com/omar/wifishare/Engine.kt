package com.omar.wifishare

import android.annotation.SuppressLint
import android.content.Context
import android.content.SharedPreferences
import android.media.MediaScannerConnection
import android.net.wifi.WifiManager
import android.os.Build
import android.os.Environment
import android.provider.Settings
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File

/** The app's view of the engine: its state, polled, plus the phone's settings. */
object Engine {
    private lateinit var app: Context
    private lateinit var prefs: SharedPreferences
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val _state = MutableStateFlow(EngineState.EMPTY)
    val state: StateFlow<EngineState> = _state

    /** Why the engine could not start, if it could not. */
    val startError = MutableStateFlow<String?>(null)

    /** Copies already handed to the media scanner. */
    private val scanned = mutableSetOf<Long>()
    private val started = mutableMapOf<Long, Long>()

    @SuppressLint("WifiManagerLeak")
    fun init(context: Context) {
        app = context.applicationContext
        prefs = app.getSharedPreferences("wifishare", Context.MODE_PRIVATE)
        // Computers announce themselves by broadcast, which Wi-Fi drivers drop without this.
        val wifi = app.getSystemService(Context.WIFI_SERVICE) as WifiManager
        wifi.createMulticastLock("wifishare").apply { setReferenceCounted(false) }.acquire()
        apply()
        scope.launch {
            while (true) {
                val st = refresh()
                val busy = st.prompts.isNotEmpty() || st.copies.any { !it.finished } || st.link.state == "searching"
                delay(if (busy) 300 else 1000)
            }
        }
    }

    /** Starts the engine, or hands it changed settings. */
    fun apply() {
        startError.value = Native.start(app.filesDir.path, phoneName, storageRoot()?.path, allowAdd)
    }

    fun refresh(): EngineState {
        val st = EngineState.parse(Native.state())
        _state.value = st
        scanFinished(st)
        return st
    }

    var phoneName: String
        get() = prefs.getString("name", null) ?: defaultName()
        set(v) {
            prefs.edit().putString("name", v.trim().ifEmpty { defaultName() }).apply()
            apply()
        }

    /** Computers may add files to this phone (never overwrite or delete). */
    var allowAdd: Boolean
        get() = prefs.getBoolean("allowAdd", true)
        set(v) {
            prefs.edit().putBoolean("allowAdd", v).apply()
            apply()
        }

    /** Keep the link (and copies) going while the app is not on screen. */
    var background: Boolean
        get() = prefs.getBoolean("background", true)
        set(v) {
            prefs.edit().putBoolean("background", v).apply()
        }

    private fun defaultName(): String =
        Settings.Global.getString(app.contentResolver, Settings.Global.DEVICE_NAME)?.takeIf { it.isNotBlank() } ?: Build.MODEL

    fun hasAllFiles(): Boolean = Environment.isExternalStorageManager()

    /** What computers browse: the whole shared storage, once the app may read it. */
    fun storageRoot(): File? = if (hasAllFiles()) Environment.getExternalStorageDirectory() else null

    /** Where copies from a computer land. */
    fun downloads(): File =
        if (hasAllFiles()) Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS)
        else app.getExternalFilesDir(Environment.DIRECTORY_DOWNLOADS) ?: app.filesDir

    suspend fun <T> io(block: () -> T): T = withContext(Dispatchers.IO) { block() }

    /** Runs a call that returns an error or null, then refreshes the state. */
    suspend fun act(block: () -> String?): String? = io {
        val e = block()
        refresh()
        e
    }

    /** Lets the gallery and other apps see files copied to Download. */
    private fun scanFinished(st: EngineState) {
        val now = System.currentTimeMillis()
        for (c in st.copies) {
            if (!c.download) continue
            started.getOrPut(c.id) { now }
            if (c.state != "done" || !scanned.add(c.id)) continue
            val since = (started[c.id] ?: now) - 5_000
            val fresh = File(c.place).walkTopDown().maxDepth(6).filter { it.isFile && it.lastModified() >= since }.map { it.path }.toList()
            if (fresh.isNotEmpty()) MediaScannerConnection.scanFile(app, fresh.toTypedArray(), null, null)
        }
    }
}

package com.omar.smsmanager.data

import android.content.Context
import android.util.Log
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext

class ConvRow(val conv: Conversation, val name: String?) {
    val title get() = name ?: conv.address
}

/**
 * The conversation list, kept in memory for the life of the process. Screens come and go on every tab
 * switch; this stays, so the list is on screen immediately and refreshes behind it instead of starting empty.
 */
object ConversationCache {
    /** Null only until the first load finishes. */
    val rows = MutableStateFlow<List<ConvRow>?>(null)
    private val lock = Mutex()

    suspend fun refresh(ctx: Context) = lock.withLock {
        val t0 = System.nanoTime()
        val convs = withContext(Dispatchers.IO) { SmsStore.conversations(ctx) }
        val tList = System.nanoTime()
        // Show the list now with whatever names are already known; looking up the rest can take a while.
        rows.value = convs.map { ConvRow(it, Contacts.peek(it.address)) }
        rows.value = withContext(Dispatchers.IO) { convs.map { ConvRow(it, Contacts.name(ctx, it.address)) } }
        Log.d("SmsManager", "conversations: ${convs.size} threads, list ${(tList - t0) / 1_000_000} ms, names ${(System.nanoTime() - tList) / 1_000_000} ms")
    }
}

package com.omar.smsmanager.data

import kotlinx.coroutines.flow.update

import android.provider.Telephony

data class Conversation(
    val threadId: Long,
    val address: String,
    val snippet: String,
    val date: Long,
    val unread: Int,
    val lastOutgoing: Boolean,
)

data class Message(
    val id: Long,
    val address: String,
    val body: String,
    val date: Long,
    val type: Int,
    val read: Boolean,
) {
    val outgoing: Boolean get() = type != Telephony.Sms.MESSAGE_TYPE_INBOX
    val failed: Boolean get() = type == Telephony.Sms.MESSAGE_TYPE_FAILED
    val sending: Boolean
        get() = type == Telephony.Sms.MESSAGE_TYPE_OUTBOX || type == Telephony.Sms.MESSAGE_TYPE_QUEUED
}

data class BlockedMessage(
    val id: Long,
    val sender: String,
    val body: String,
    val date: Long,
    val ruleId: Long,
    val ruleLabel: String,
)

data class OutboxItem(val id: Long, val sender: String, val body: String, val date: Long)

/** Bumped whenever the SMS provider or our own database changes; screens reload on it. */
object Ticker {
    val tick = kotlinx.coroutines.flow.MutableStateFlow(0)
    fun bump() = tick.update { it + 1 }
}

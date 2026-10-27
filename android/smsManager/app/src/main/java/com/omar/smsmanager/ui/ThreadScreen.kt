package com.omar.smsmanager.ui

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.Manifest
import android.content.pm.PackageManager
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.WindowInsetsSides
import androidx.compose.foundation.layout.only
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.Send
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.omar.smsmanager.data.Contacts
import com.omar.smsmanager.data.Db
import com.omar.smsmanager.data.Message
import com.omar.smsmanager.data.SmsStore
import com.omar.smsmanager.data.Ticker
import com.omar.smsmanager.sms.ClipboardCopy
import com.omar.smsmanager.sms.Notifier
import com.omar.smsmanager.sms.OtpExtractor
import com.omar.smsmanager.sms.SmsSender
import com.omar.smsmanager.spam.Rule
import com.omar.smsmanager.spam.RuleAction
import com.omar.smsmanager.spam.RuleTarget
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

private class ThreadData(val messages: List<Message>, val threadId: Long, val loaded: Boolean = true)

fun hasSendPermission(ctx: Context) =
    ctx.checkSelfPermission(Manifest.permission.SEND_SMS) == PackageManager.PERMISSION_GRANTED

/** Block this exact number, unless an identical rule is already there. */
fun blockNumber(ctx: Context, address: String) {
    val db = Db.get(ctx)
    if (!db.hasRule(RuleAction.BLOCK, RuleTarget.SENDER, address)) {
        db.saveRule(Rule(action = RuleAction.BLOCK, target = RuleTarget.SENDER, pattern = address))
    }
}

@OptIn(ExperimentalMaterial3Api::class, ExperimentalFoundationApi::class)
@Composable
fun ThreadScreen(threadId: Long, address: String, onBack: () -> Unit) {
    val ctx = LocalContext.current
    val scope = rememberCoroutineScope()
    val tick by Ticker.tick.collectAsState()
    var draft by remember { mutableStateOf("") }
    var menu by remember { mutableStateOf(false) }
    var selected by remember { mutableStateOf<Message?>(null) }
    var confirmDelete by remember { mutableStateOf(false) }

    val name by produceState<String?>(null, address) { value = withContext(Dispatchers.IO) { Contacts.name(ctx, address) } }
    val data by produceState(ThreadData(emptyList(), threadId, loaded = false), tick) {
        value = withContext(Dispatchers.IO) {
            val tid = if (threadId > 0) threadId else SmsStore.findThread(ctx, address)
            ThreadData(if (tid > 0) SmsStore.messages(ctx, tid) else emptyList(), tid)
        }
    }

    LaunchedEffect(data) {
        if (data.threadId > 0 && data.messages.any { !it.outgoing && !it.read }) {
            withContext(Dispatchers.IO) {
                SmsStore.markRead(ctx, data.threadId)
                Notifier.cancel(ctx, data.threadId, address)
            }
        }
    }

    fun send() {
        val text = draft.trim()
        if (text.isEmpty()) return
        if (!hasSendPermission(ctx)) return toast(ctx, "Grant the SMS permission first (Messages tab)")
        draft = ""
        scope.launch(Dispatchers.IO) { SmsSender.send(ctx, address, text) }
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = {
                    Column {
                        Text(name ?: address, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        if (name != null) Text(address, style = MaterialTheme.typography.bodySmall)
                    }
                },
                navigationIcon = { IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, "Back") } },
                actions = {
                    IconButton(onClick = { menu = true }) { Icon(Icons.Filled.MoreVert, "More") }
                    DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                        DropdownMenuItem(text = { Text("Block this number") }, onClick = {
                            menu = false
                            scope.launch(Dispatchers.IO) { blockNumber(ctx, address) }
                            toast(ctx, "Blocked $address")
                        })
                        if (data.threadId > 0) {
                            DropdownMenuItem(text = { Text("Delete conversation") }, onClick = { menu = false; confirmDelete = true })
                        }
                    }
                },
            )
        },
        bottomBar = {
            Row(
                Modifier.fillMaxWidth()
                    .windowInsetsPadding(WindowInsets.safeDrawing.only(WindowInsetsSides.Bottom))
                    .padding(8.dp),
                verticalAlignment = Alignment.Bottom,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                OutlinedTextField(
                    value = draft,
                    onValueChange = { draft = it },
                    placeholder = { Text("Message") },
                    maxLines = 5,
                    modifier = Modifier.weight(1f),
                )
                IconButton(onClick = ::send, enabled = draft.isNotBlank()) { Icon(Icons.AutoMirrored.Filled.Send, "Send") }
            }
        },
    ) { pad ->
        if (!data.loaded) {
            Box(Modifier.fillMaxSize().padding(pad)) {}
        } else if (data.messages.isEmpty()) {
            EmptyState("No messages yet", Modifier.padding(pad))
        } else {
            LazyColumn(
                Modifier.fillMaxSize().padding(pad),
                reverseLayout = true,
                contentPadding = androidx.compose.foundation.layout.PaddingValues(horizontal = 12.dp, vertical = 8.dp),
                verticalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                items(data.messages.asReversed(), key = { it.id }) { m ->
                    Bubble(
                        m,
                        onClick = {
                            if (m.failed) scope.launch(Dispatchers.IO) {
                                SmsStore.deleteMessage(ctx, m.id)
                                SmsSender.send(ctx, address, m.body)
                            }
                        },
                        onLongClick = { selected = m },
                    )
                }
            }
        }
    }

    selected?.let { m ->
        AlertDialog(
            onDismissRequest = { selected = null },
            title = { Text(formatFull(ctx, m.date)) },
            text = {
                Column {
                    TextButton(onClick = {
                        ctx.getSystemService(ClipboardManager::class.java).setPrimaryClip(ClipData.newPlainText("message", m.body))
                        selected = null
                    }) { Text("Copy text") }
                    TextButton(onClick = {
                        selected = null
                        scope.launch(Dispatchers.IO) { SmsStore.deleteMessage(ctx, m.id) }
                    }) { Text("Delete") }
                    if (!m.outgoing) {
                        TextButton(onClick = {
                            selected = null
                            scope.launch(Dispatchers.IO) { blockNumber(ctx, address) }
                            toast(ctx, "Blocked $address")
                        }) { Text("Block this number") }
                    }
                }
            },
            confirmButton = { TextButton(onClick = { selected = null }) { Text("Close") } },
        )
    }

    if (confirmDelete) {
        AlertDialog(
            onDismissRequest = { confirmDelete = false },
            title = { Text("Delete conversation?") },
            confirmButton = {
                TextButton(onClick = {
                    confirmDelete = false
                    scope.launch(Dispatchers.IO) { SmsStore.deleteThread(ctx, data.threadId) }
                    onBack()
                }) { Text("Delete") }
            },
            dismissButton = { TextButton(onClick = { confirmDelete = false }) { Text("Cancel") } },
        )
    }
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun Bubble(m: Message, onClick: () -> Unit, onLongClick: () -> Unit) {
    val ctx = LocalContext.current
    Box(Modifier.fillMaxWidth(), contentAlignment = if (m.outgoing) Alignment.CenterEnd else Alignment.CenterStart) {
        Surface(
            shape = RoundedCornerShape(16.dp),
            color = when {
                m.failed -> MaterialTheme.colorScheme.errorContainer
                m.outgoing -> MaterialTheme.colorScheme.primaryContainer
                else -> MaterialTheme.colorScheme.surfaceVariant
            },
            modifier = Modifier.widthIn(max = 300.dp).combinedClickable(onClick = onClick, onLongClick = onLongClick),
        ) {
            Column(Modifier.padding(horizontal = 12.dp, vertical = 8.dp)) {
                Text(m.body)
                val code = remember(m.body) { if (m.outgoing) null else OtpExtractor.extract(m.body) }
                if (code != null) {
                    TextButton(onClick = {
                        ClipboardCopy.copyCode(ctx, code)
                        toast(ctx, "Copied $code")
                    }) { Text("Copy $code") }
                }
                val status = when {
                    m.failed -> "Not sent. Tap to retry"
                    m.sending -> "Sending…"
                    else -> formatWhen(ctx, m.date)
                }
                Text(status, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
    }
}

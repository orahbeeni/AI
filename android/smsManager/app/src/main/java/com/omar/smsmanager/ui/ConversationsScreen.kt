package com.omar.smsmanager.ui

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Search
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
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
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.omar.smsmanager.data.Contacts
import com.omar.smsmanager.data.ConvRow
import com.omar.smsmanager.data.ConversationCache
import com.omar.smsmanager.spam.PatternMatcher
import com.omar.smsmanager.data.Conversation
import com.omar.smsmanager.data.SmsStore
import com.omar.smsmanager.data.Ticker
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

private class HitRow(val hit: SmsStore.Hit, val name: String?)

@OptIn(ExperimentalFoundationApi::class)
@Composable
fun ConversationsScreen(onOpenThread: (Long, String) -> Unit, onCompose: () -> Unit) {
    val ctx = LocalContext.current
    val scope = rememberCoroutineScope()
    val tick by Ticker.tick.collectAsState()
    // Deliberately not rememberSaveable: a search typed before the app was killed in the background
    // would come back and leave the list looking mostly empty.
    var query by remember { mutableStateOf("") }
    var toDelete by remember { mutableStateOf<Conversation?>(null) }

    val rows by ConversationCache.rows.collectAsState()
    val hits by produceState(emptyList<HitRow>(), query, tick) {
        if (query.trim().length < 2) {
            value = emptyList()
        } else {
            delay(250) // debounce typing
            value = withContext(Dispatchers.IO) { SmsStore.search(ctx, query.trim()).map { HitRow(it, Contacts.name(ctx, it.address)) } }
        }
    }

    val q = query.trim()
    val matches = remember(q) { PatternMatcher.searchMatcher(q) }
    val shown = rows?.filter { q.isEmpty() || matches(it.title) || matches(it.conv.address) }

    Box(Modifier.fillMaxSize()) {
        androidx.compose.foundation.layout.Column(Modifier.fillMaxSize()) {
            OutlinedTextField(
                value = query,
                onValueChange = { query = it },
                placeholder = { Text("Search  (* any text, ? one character)") },
                leadingIcon = { Icon(Icons.Filled.Search, null) },
                singleLine = true,
                modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp),
            )
            when {
                shown == null -> Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { CircularProgressIndicator() }
                shown.isEmpty() && hits.isEmpty() -> EmptyState(if (q.isEmpty()) "No messages yet" else "No matches")
                else -> LazyColumn(Modifier.fillMaxSize()) {
                    items(shown, key = { it.conv.threadId }) { row ->
                        val c = row.conv
                        ListItem(
                            modifier = Modifier.combinedClickable(
                                onClick = { onOpenThread(c.threadId, c.address) },
                                onLongClick = { toDelete = c },
                            ),
                            leadingContent = { Avatar(row.title) },
                            headlineContent = {
                                Text(row.title, maxLines = 1, overflow = TextOverflow.Ellipsis, fontWeight = if (c.unread > 0) FontWeight.Bold else null)
                            },
                            supportingContent = {
                                Text(
                                    (if (c.lastOutgoing) "You: " else "") + c.snippet,
                                    maxLines = 1, overflow = TextOverflow.Ellipsis,
                                    fontWeight = if (c.unread > 0) FontWeight.Bold else null,
                                )
                            },
                            trailingContent = { Text(formatWhen(ctx, c.date)) },
                        )
                    }
                    if (hits.isNotEmpty()) {
                        item { Text("Matching messages", Modifier.padding(16.dp, 12.dp, 16.dp, 4.dp), style = androidx.compose.material3.MaterialTheme.typography.labelLarge) }
                        items(hits) { h ->
                            ListItem(
                                modifier = Modifier.combinedClickable(onClick = { onOpenThread(h.hit.threadId, h.hit.address) }),
                                headlineContent = { Text(h.name ?: h.hit.address, maxLines = 1) },
                                supportingContent = { Text(h.hit.body, maxLines = 2, overflow = TextOverflow.Ellipsis) },
                                trailingContent = { Text(formatWhen(ctx, h.hit.date)) },
                            )
                            HorizontalDivider()
                        }
                    }
                }
            }
        }
        FloatingActionButton(onClick = onCompose, modifier = Modifier.align(Alignment.BottomEnd).padding(16.dp)) {
            Icon(Icons.Filled.Add, "New message")
        }
    }

    toDelete?.let { c ->
        AlertDialog(
            onDismissRequest = { toDelete = null },
            title = { Text("Delete conversation?") },
            text = { Text("All messages with ${Contacts.name(ctx, c.address) ?: c.address} will be deleted.") },
            confirmButton = {
                TextButton(onClick = {
                    toDelete = null
                    scope.launch(Dispatchers.IO) { SmsStore.deleteThread(ctx, c.threadId) }
                }) { Text("Delete") }
            },
            dismissButton = { TextButton(onClick = { toDelete = null }) { Text("Cancel") } },
        )
    }
}

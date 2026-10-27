package com.omar.smsmanager.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.Card
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.produceState
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import com.omar.smsmanager.data.BlockedMessage
import com.omar.smsmanager.data.Db
import com.omar.smsmanager.data.SmsStore
import com.omar.smsmanager.data.Ticker
import com.omar.smsmanager.spam.Rule
import com.omar.smsmanager.spam.RuleAction
import com.omar.smsmanager.spam.RuleTarget
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

private object LastSeen {
    @Volatile var blocked: List<BlockedMessage>? = null
}

@Composable
fun BlockedScreen() {
    val ctx = LocalContext.current
    val scope = rememberCoroutineScope()
    val tick by Ticker.tick.collectAsState()
    // Starts from the last list shown, so switching tabs never flashes "Nothing blocked" while loading.
    val loaded by produceState(LastSeen.blocked, tick) {
        value = withContext(Dispatchers.IO) { Db.get(ctx).blocked() }.also { LastSeen.blocked = it }
    }
    val items = loaded ?: return

    if (items.isEmpty()) return EmptyState("Nothing blocked. Messages that match a rule are held here instead of reaching your inbox.")

    Column(Modifier.fillMaxSize()) {
        Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp), verticalAlignment = Alignment.CenterVertically) {
            Text("${items.size} held", Modifier.weight(1f), style = MaterialTheme.typography.labelLarge)
            TextButton(onClick = { scope.launch(Dispatchers.IO) { Db.get(ctx).clearBlocked() } }) { Text("Delete all") }
        }
        LazyColumn(Modifier.fillMaxSize(), verticalArrangement = Arrangement.spacedBy(8.dp), contentPadding = androidx.compose.foundation.layout.PaddingValues(16.dp, 0.dp, 16.dp, 16.dp)) {
            items(items, key = { it.id }) { b ->
                Card(Modifier.fillMaxWidth()) {
                    Column(Modifier.padding(12.dp)) {
                        Row {
                            Text(b.sender, Modifier.weight(1f), style = MaterialTheme.typography.titleSmall)
                            Text(formatWhen(ctx, b.date), style = MaterialTheme.typography.labelSmall)
                        }
                        Text(b.body, Modifier.padding(vertical = 6.dp))
                        Text("Matched ${b.ruleLabel}", style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                        Row {
                            TextButton(onClick = {
                                scope.launch(Dispatchers.IO) {
                                    // A false positive: put it back and make sure this sender is let through next time.
                                    SmsStore.insertInbox(ctx, b.sender, b.body, b.date, -1, read = false)
                                    val db = Db.get(ctx)
                                    if (!db.hasRule(RuleAction.ALLOW, RuleTarget.SENDER, b.sender)) {
                                        db.saveRule(Rule(action = RuleAction.ALLOW, target = RuleTarget.SENDER, pattern = b.sender))
                                    }
                                    db.deleteBlocked(b.id)
                                }
                            }) { Text("Not spam") }
                            TextButton(onClick = { scope.launch(Dispatchers.IO) { Db.get(ctx).deleteBlocked(b.id) } }) { Text("Delete") }
                        }
                    }
                }
            }
        }
    }
}

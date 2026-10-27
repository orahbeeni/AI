package com.omar.smsmanager.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExtendedFloatingActionButton
import androidx.compose.material3.FilterChip
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Switch
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
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import com.omar.smsmanager.data.Db
import com.omar.smsmanager.data.Settings
import com.omar.smsmanager.sms.DefaultApp
import com.omar.smsmanager.data.Ticker
import com.omar.smsmanager.spam.Candidate
import com.omar.smsmanager.spam.MatchMode
import com.omar.smsmanager.spam.PatternMatcher
import com.omar.smsmanager.spam.Retro
import com.omar.smsmanager.spam.ScanRange
import com.omar.smsmanager.spam.Rule
import com.omar.smsmanager.spam.RuleAction
import com.omar.smsmanager.spam.RuleTarget
import com.omar.smsmanager.spam.SpamFilter
import com.omar.smsmanager.spam.Verdict
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

private object LastSeenRules {
    @Volatile var rules: List<Rule>? = null
}

@Composable
fun RulesScreen() {
    val ctx = LocalContext.current
    val scope = rememberCoroutineScope()
    val tick by Ticker.tick.collectAsState()
    // Starts from the last list shown, so switching tabs never flashes "No rules yet" while loading.
    val loaded by produceState(LastSeenRules.rules, tick) {
        value = withContext(Dispatchers.IO) { Db.get(ctx).rules() }.also { LastSeenRules.rules = it }
    }
    val rules = loaded ?: emptyList()
    var editing by remember { mutableStateOf<Rule?>(null) }
    var creating by remember { mutableStateOf(false) }
    var testing by remember { mutableStateOf(false) }
    var scanning by remember { mutableStateOf(false) }

    Box(Modifier.fillMaxSize()) {
        Column(Modifier.fillMaxSize()) {
            Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp), verticalAlignment = Alignment.CenterVertically) {
                Text(
                    "Allow rules win, then contacts, then block rules.",
                    Modifier.weight(1f), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                TextButton(onClick = { testing = true }) { Text("Test") }
                TextButton(onClick = { scanning = true }) { Text("Scan old") }
            }
            if (loaded == null) {
                Box(Modifier.fillMaxSize()) {}
            } else if (rules.isEmpty()) {
                EmptyState("No rules yet. Add one with the button below, or block a number from a conversation.")
            } else {
                LazyColumn(Modifier.fillMaxSize(), contentPadding = androidx.compose.foundation.layout.PaddingValues(bottom = 88.dp)) {
                    items(rules, key = { it.id }) { r ->
                        ListItem(
                            modifier = Modifier.clickable { editing = r },
                            headlineContent = { Text(r.pattern, fontFamily = FontFamily.Monospace) },
                            supportingContent = {
                                Text(
                                    listOf(
                                        if (r.action == RuleAction.BLOCK) "Block" else "Allow",
                                        if (r.target == RuleTarget.SENDER) "sender" else "text",
                                        r.mode.name.lowercase(),
                                        "${r.hits} ${if (r.hits == 1) "hit" else "hits"}",
                                    ).joinToString(" · ")
                                )
                            },
                            trailingContent = {
                                Switch(checked = r.enabled, onCheckedChange = { on ->
                                    scope.launch(Dispatchers.IO) { Db.get(ctx).setRuleEnabled(r.id, on) }
                                })
                            },
                        )
                        HorizontalDivider()
                    }
                }
            }
        }
        ExtendedFloatingActionButton(
            onClick = { creating = true },
            icon = { Icon(Icons.Filled.Add, null) },
            text = { Text("Add rule") },
            modifier = Modifier.align(Alignment.BottomEnd).padding(16.dp),
        )
    }

    if (creating || editing != null) {
        RuleEditor(
            initial = editing,
            onDismiss = { creating = false; editing = null },
            onSave = { rule ->
                creating = false; editing = null
                scope.launch(Dispatchers.IO) { Db.get(ctx).saveRule(rule) }
            },
            onDelete = { rule ->
                editing = null
                scope.launch(Dispatchers.IO) { Db.get(ctx).deleteRule(rule.id) }
            },
        )
    }
    if (testing) RuleTester(rules, onDismiss = { testing = false })
    if (scanning) ScanDialog(onDismiss = { scanning = false })
}

@Composable
private fun RuleEditor(initial: Rule?, onDismiss: () -> Unit, onSave: (Rule) -> Unit, onDelete: (Rule) -> Unit) {
    var action by remember { mutableStateOf(initial?.action ?: RuleAction.BLOCK) }
    var target by remember { mutableStateOf(initial?.target ?: RuleTarget.TEXT) }
    var pattern by remember { mutableStateOf(initial?.pattern.orEmpty()) }
    var mode by remember { mutableStateOf(initial?.mode ?: MatchMode.WILDCARD) }
    val valid = PatternMatcher.isValid(pattern, mode, target)

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(if (initial == null) "New rule" else "Edit rule") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    FilterChip(action == RuleAction.BLOCK, { action = RuleAction.BLOCK }, { Text("Block") })
                    FilterChip(action == RuleAction.ALLOW, { action = RuleAction.ALLOW }, { Text("Allow") })
                }
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    FilterChip(target == RuleTarget.SENDER, { target = RuleTarget.SENDER }, { Text("Sender") })
                    FilterChip(target == RuleTarget.TEXT, { target = RuleTarget.TEXT }, { Text("Message text") })
                }
                OutlinedTextField(
                    value = pattern,
                    onValueChange = { pattern = it },
                    label = { Text(if (target == RuleTarget.SENDER) "Number or name" else "Words to look for") },
                    isError = pattern.isNotEmpty() && !valid,
                    supportingText = { if (pattern.isNotEmpty() && !valid) Text("Not a valid pattern") },
                    modifier = Modifier.fillMaxWidth(),
                )
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    FilterChip(mode == MatchMode.WILDCARD, { mode = MatchMode.WILDCARD }, { Text("Wildcard") })
                    FilterChip(mode == MatchMode.LOOSE, { mode = MatchMode.LOOSE }, { Text("Loose") })
                    FilterChip(mode == MatchMode.REGEX, { mode = MatchMode.REGEX }, { Text("Regex") })
                }
                Text(
                    when (mode) {
                        MatchMode.REGEX -> "Java regular expression, ignoring case."
                        MatchMode.LOOSE ->
                            "* = any text, ? = zero or one character. Spaces, punctuation, accents and case are ignored, " +
                                "so opt?out matches Opt-out, O.P.T out and optout."
                        MatchMode.WILDCARD ->
                            if (target == RuleTarget.SENDER) "* = anything, ? = one character. Matches the whole number, e.g. +1555* or *4567. Spaces and dashes are ignored."
                            else "* = anything, ? = one character, \\* = a literal star. Matches anywhere in the message, e.g. free gift or click*http."
                    },
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        },
        confirmButton = {
            TextButton(enabled = valid, onClick = {
                onSave(
                    (initial ?: Rule(action = action, target = target, pattern = pattern))
                        .copy(action = action, target = target, pattern = pattern.trim(), mode = mode)
                )
            }) { Text("Save") }
        },
        dismissButton = {
            Row {
                if (initial != null) TextButton(onClick = { onDelete(initial) }) { Text("Delete") }
                TextButton(onClick = onDismiss) { Text("Cancel") }
            }
        },
    )
}

/** Runs a made-up message through the current rules so patterns can be checked without waiting for spam. */
@Composable
private fun RuleTester(rules: List<Rule>, onDismiss: () -> Unit) {
    var sender by remember { mutableStateOf("") }
    var text by remember { mutableStateOf("") }
    val verdict = SpamFilter(rules).evaluate(sender, text)
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Test your rules") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedTextField(sender, { sender = it }, label = { Text("Sender") }, singleLine = true, modifier = Modifier.fillMaxWidth())
                OutlinedTextField(text, { text = it }, label = { Text("Message") }, minLines = 2, modifier = Modifier.fillMaxWidth())
                Text(
                    when (verdict) {
                        is Verdict.Blocked -> "Blocked by ${verdict.rule.label}"
                        is Verdict.Allowed -> "Allowed" + (verdict.rule?.let { " by ${it.label}" } ?: "")
                        Verdict.Pass -> "Delivered. No rule matches."
                    },
                    style = MaterialTheme.typography.titleSmall,
                )
                Text("Contacts aren't considered here.", style = MaterialTheme.typography.bodySmall)
            }
        },
        confirmButton = { TextButton(onClick = onDismiss) { Text("Close") } },
    )
}

private sealed interface ScanState {
    data object Idle : ScanState
    data object Working : ScanState
    class Found(val items: List<Candidate>) : ScanState
    class Done(val moved: Int, val failed: Boolean) : ScanState
}

/**
 * Applies the current rules to messages already in the inbox. Nothing moves until the preview is confirmed,
 * and moved messages go to Blocked (kept per the retention setting) rather than being deleted outright.
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun ScanDialog(onDismiss: () -> Unit) {
    val ctx = LocalContext.current
    val scope = rememberCoroutineScope()
    var range by remember { mutableStateOf(ScanRange.MONTH) }
    var state by remember { mutableStateOf<ScanState>(ScanState.Idle) }
    val isDefault = remember { DefaultApp.isDefault(ctx) }
    val retention = remember { Settings(ctx).retentionDays }

    AlertDialog(
        onDismissRequest = { if (state !is ScanState.Working) onDismiss() },
        title = { Text("Scan old messages") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                when (val st = state) {
                    ScanState.Idle -> {
                        Text("Check received messages against your rules. You'll see what matches before anything moves.")
                        FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            ScanRange.entries.forEach { r -> FilterChip(range == r, { range = r }, { Text(r.label) }) }
                        }
                    }
                    ScanState.Working -> {
                        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                            CircularProgressIndicator(Modifier.padding(4.dp))
                            Text("Working…")
                        }
                    }
                    is ScanState.Found -> {
                        if (st.items.isEmpty()) {
                            Text("No messages from the last ${range.label.lowercase()} match your rules.")
                        } else {
                            Text("${st.items.size} message${if (st.items.size == 1) "" else "s"} from ${st.items.map { it.sender }.toSet().size} sender(s) match:", style = MaterialTheme.typography.titleSmall)
                            st.items.groupBy { it.rule.id }.values.sortedByDescending { it.size }.take(6).forEach { g ->
                                Text("${g.size} × ${g.first().rule.label}", fontFamily = FontFamily.Monospace, style = MaterialTheme.typography.bodySmall)
                            }
                            Text(
                                if (retention == 0) "They will move to Blocked and be kept there."
                                else "They will move to Blocked and be deleted for good after $retention days.",
                                style = MaterialTheme.typography.bodySmall,
                            )
                            if (!isDefault) Text("Make SmsManager the default SMS app first. Only the default app can remove messages.", color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall)
                        }
                    }
                    is ScanState.Done -> Text(
                        if (st.failed) "Moved ${st.moved}. Some messages couldn't be removed from the inbox, but they are all safe in Blocked."
                        else "Moved ${st.moved} message${if (st.moved == 1) "" else "s"} to Blocked.",
                    )
                }
            }
        },
        confirmButton = {
            when (val st = state) {
                ScanState.Idle -> TextButton(onClick = {
                    state = ScanState.Working
                    scope.launch { state = ScanState.Found(withContext(Dispatchers.IO) { Retro.scan(ctx, range) }) }
                }) { Text("Scan") }
                ScanState.Working -> {}
                is ScanState.Found -> if (st.items.isNotEmpty()) TextButton(enabled = isDefault, onClick = {
                    state = ScanState.Working
                    scope.launch {
                        val o = withContext(Dispatchers.IO) { Retro.apply(ctx, st.items) }
                        state = ScanState.Done(o.moved, o.failed)
                    }
                }) { Text("Move to Blocked") }
                is ScanState.Done -> TextButton(onClick = onDismiss) { Text("Close") }
            }
        },
        dismissButton = {
            when (state) {
                is ScanState.Found -> TextButton(onClick = { state = ScanState.Idle }) { Text("Back") }
                ScanState.Idle -> TextButton(onClick = onDismiss) { Text("Cancel") }
                else -> {}
            }
        },
    )
}

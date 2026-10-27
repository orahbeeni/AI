package com.omar.smsmanager.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.unit.dp
import com.omar.smsmanager.data.Db
import com.omar.smsmanager.data.SmsStore
import com.omar.smsmanager.spam.MatchMode
import com.omar.smsmanager.spam.PatternMatcher
import com.omar.smsmanager.spam.Rule
import com.omar.smsmanager.spam.RuleAction
import com.omar.smsmanager.spam.RuleTarget
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * Turns part of a message into a rule. The message is shown as selectable text: touch and hold a word,
 * drag the handles, and the selection becomes the pattern. A live line shows whether the rule would catch
 * this message and how many messages already in the inbox it matches, which is the check against a rule
 * that is too broad.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun RuleFromMessageSheet(sender: String, body: String, onDismiss: () -> Unit) {
    val ctx = LocalContext.current
    val scope = rememberCoroutineScope()
    val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)

    var field by remember { mutableStateOf(TextFieldValue(body)) }
    var pattern by remember { mutableStateOf("") }
    var edited by remember { mutableStateOf(false) } // once the user types in the pattern box, stop following the selection
    var action by remember { mutableStateOf(RuleAction.BLOCK) }
    var mode by remember { mutableStateOf(MatchMode.WILDCARD) }

    fun follow(sel: TextRange) {
        if (edited) return
        val picked = body.substring(sel.min, sel.max).trim()
        pattern = PatternMatcher.escapeWildcards(picked)
    }

    val compiled = remember(pattern, mode) { PatternMatcher.compile(pattern, mode, RuleTarget.TEXT) }
    val catchesThis = remember(compiled, mode, body) {
        compiled?.containsMatchIn(if (mode == MatchMode.LOOSE) PatternMatcher.looseNormalize(body) else body) ?: false
    }
    // How many messages already in the inbox this wildcard rule would match. Loose mode isn't counted exactly.
    val inboxMatches by produceState<Int?>(null, pattern, mode) {
        value = null
        if (compiled == null || mode != MatchMode.WILDCARD) return@produceState
        delay(300) // debounce while dragging the handles
        value = withContext(Dispatchers.IO) { SmsStore.search(ctx, pattern, 101).size }
    }

    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = sheetState) {
        Column(
            Modifier.verticalScroll(rememberScrollState()).navigationBarsPadding().padding(horizontal = 20.dp).padding(bottom = 16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Text("Make a rule from this message", style = MaterialTheme.typography.titleLarge)
            Text(
                "Touch and hold a word, then drag the handles to select the part that should trigger the rule.",
                style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant,
            )

            Surface(shape = RoundedCornerShape(12.dp), color = MaterialTheme.colorScheme.surfaceVariant, modifier = Modifier.fillMaxWidth()) {
                BasicTextField(
                    value = field,
                    onValueChange = { new ->
                        field = field.copy(selection = new.selection) // the text itself is fixed
                        follow(new.selection)
                    },
                    readOnly = true,
                    textStyle = MaterialTheme.typography.bodyLarge.copy(color = MaterialTheme.colorScheme.onSurfaceVariant),
                    cursorBrush = SolidColor(androidx.compose.ui.graphics.Color.Transparent),
                    modifier = Modifier.padding(14.dp).fillMaxWidth(),
                )
            }

            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                FilterChip(action == RuleAction.BLOCK, { action = RuleAction.BLOCK }, { Text("Block") })
                FilterChip(action == RuleAction.ALLOW, { action = RuleAction.ALLOW }, { Text("Allow") })
                FilterChip(mode == MatchMode.WILDCARD, { mode = MatchMode.WILDCARD }, { Text("Exact words") })
                FilterChip(mode == MatchMode.LOOSE, { mode = MatchMode.LOOSE }, { Text("Loose") })
            }

            OutlinedTextField(
                value = pattern,
                onValueChange = { pattern = it; edited = true },
                label = { Text("Words that trigger the rule") },
                placeholder = { Text("Select text above, or type here") },
                supportingText = { Text("You can edit this. * is any text, ? is one character.") },
                isError = pattern.isNotBlank() && compiled == null,
                modifier = Modifier.fillMaxWidth(),
            )

            // Live preview
            val preview = when {
                pattern.isBlank() -> "Nothing selected yet."
                compiled == null -> "That isn't a valid pattern."
                !catchesThis -> "This pattern would not catch this message."
                mode != MatchMode.WILDCARD -> "Catches this message. Loose matching ignores spaces and punctuation, so it can match more."
                inboxMatches == null -> "Catches this message. Checking your inbox…"
                inboxMatches!! > 100 -> "Catches this message and 100+ in your inbox. That is very broad."
                else -> "Catches this message and matches ${inboxMatches} message${if (inboxMatches == 1) "" else "s"} in your inbox."
            }
            val broad = (inboxMatches ?: 0) > 20
            Text(
                preview,
                style = MaterialTheme.typography.bodyMedium,
                color = if (broad || (pattern.isNotBlank() && !catchesThis)) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurface,
            )

            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                TextButton(onClick = {
                    scope.launch(Dispatchers.IO) { blockNumber(ctx, sender) }
                    toast(ctx, "Blocked $sender")
                    onDismiss()
                }) { Text("Block sender instead") }
                Row(Modifier.weight(1f), horizontalArrangement = Arrangement.End) {
                    TextButton(onClick = onDismiss) { Text("Cancel") }
                    TextButton(
                        enabled = compiled != null && pattern.isNotBlank(),
                        onClick = {
                            scope.launch(Dispatchers.IO) {
                                val db = Db.get(ctx)
                                val exists = mode == MatchMode.WILDCARD && db.hasRule(action, RuleTarget.TEXT, pattern.trim())
                                if (!exists) db.saveRule(Rule(action = action, target = RuleTarget.TEXT, pattern = pattern.trim(), mode = mode))
                            }
                            toast(ctx, "Rule added. Use Rules, Scan old to apply it to older messages.")
                            onDismiss()
                        },
                    ) { Text(if (action == RuleAction.BLOCK) "Add block rule" else "Add allow rule") }
                }
            }
        }
    }
}

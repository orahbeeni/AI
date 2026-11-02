package com.omar.wifishare.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Computer
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.omar.wifishare.Engine
import com.omar.wifishare.EngineState
import com.omar.wifishare.Link
import com.omar.wifishare.Native
import com.omar.wifishare.Prompt
import kotlinx.coroutines.launch

@Composable
fun StatusDot(link: Link) {
    val color = when (link.state) {
        "connected" -> Green
        "searching" -> Amber
        else -> Color.Gray
    }
    Box(Modifier.size(8.dp).background(color, CircleShape))
}

fun span(secs: Long): String = when {
    secs < 60 -> "${secs}s"
    secs < 3600 -> "${secs / 60} min"
    else -> "${secs / 3600} h ${secs % 3600 / 60} min"
}

fun linkText(link: Link): String = when (link.state) {
    "connected" -> listOfNotNull("Connected", link.addr, link.rtt?.let { "$it ms" }).joinToString(" · ")
    "searching" -> if (link.secs < 4 || link.error == null) "Looking for it…" else "Looking for it… ${link.error}"
    "unpaired" -> "Pair a computer to start"
    else -> "Not linked"
}

/** Big centred message for an empty screen, with an optional action. */
@Composable
fun Empty(title: String, text: String, modifier: Modifier = Modifier, action: Pair<String, () -> Unit>? = null) {
    Column(
        modifier.fillMaxWidth().padding(32.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        Text(title, style = MaterialTheme.typography.titleMedium)
        Spacer(Modifier.height(8.dp))
        Text(text, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        action?.let { (label, go) ->
            Spacer(Modifier.height(16.dp))
            Button(onClick = go) { Text(label) }
        }
    }
}

/** Pairing: computers heard on this Wi-Fi, or one typed in. */
@Composable
fun PairDialog(st: EngineState, onClose: () -> Unit) {
    var address by remember { mutableStateOf("") }
    var asked by remember { mutableStateOf<String?>(null) }
    var failed by remember { mutableStateOf<String?>(null) }
    // The notice showing when the attempt started is not its outcome.
    var before by remember { mutableStateOf<Long?>(null) }
    val scope = rememberCoroutineScope()
    fun pair(target: String) {
        asked = target
        failed = null
        before = st.notice?.id
        scope.launch { Engine.act { Native.pair(target) } }
    }
    LaunchedEffect(st.notice) {
        val n = st.notice ?: return@LaunchedEffect
        if (asked != null && n.id != before && n.text.startsWith("Pairing failed")) {
            asked = null
            failed = n.text
        }
    }
    val unpaired = st.discovered.filter { !it.paired }
    AlertDialog(
        onDismissRequest = onClose,
        title = { Text("Pair a computer") },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState())) {
                Text(
                    "On the computer, open wifishare and start sharing (either mode). It shows up here; " +
                        "then compare the 6-digit codes on both screens.",
                    style = MaterialTheme.typography.bodyMedium,
                )
                Spacer(Modifier.height(12.dp))
                if (unpaired.isEmpty()) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp)
                        Spacer(Modifier.width(10.dp))
                        Text("Looking on this Wi-Fi…", color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                }
                for (h in unpaired) {
                    Row(Modifier.fillMaxWidth().padding(vertical = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                        Icon(Icons.Default.Computer, null)
                        Spacer(Modifier.width(10.dp))
                        Column(Modifier.weight(1f)) {
                            Text(h.name, fontWeight = FontWeight.SemiBold)
                            Text(h.addr.substringBefore(':'), style = MaterialTheme.typography.bodySmall)
                        }
                        OutlinedButton(onClick = { pair(h.addr) }, enabled = asked == null) { Text("Pair") }
                    }
                }
                Spacer(Modifier.height(12.dp))
                OutlinedTextField(
                    value = address,
                    onValueChange = { address = it },
                    label = { Text("Or its address") },
                    placeholder = { Text("192.168.1.25") },
                    singleLine = true,
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri, imeAction = ImeAction.Go),
                    keyboardActions = KeyboardActions(onGo = { if (address.isNotBlank()) pair(address.trim()) }),
                )
                failed?.let {
                    Spacer(Modifier.height(12.dp))
                    Text(it, color = Red, style = MaterialTheme.typography.bodyMedium)
                }
                asked?.let {
                    Spacer(Modifier.height(12.dp))
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp)
                        Spacer(Modifier.width(10.dp))
                        Text("Pairing with ${it.substringBefore(':')}…")
                    }
                }
            }
        },
        confirmButton = {
            TextButton(onClick = { pair(address.trim()) }, enabled = address.isNotBlank() && asked == null) { Text("Pair") }
        },
        dismissButton = { TextButton(onClick = onClose) { Text("Close") } },
    )
}

/** The code to compare with the computer's screen. */
@Composable
fun CodeDialog(p: Prompt) {
    val scope = rememberCoroutineScope()
    fun reply(yes: Boolean) {
        scope.launch { Engine.act { Native.answer(p.id, yes) } }
    }
    AlertDialog(
        onDismissRequest = {},
        title = { Text("Check the code") },
        text = {
            Column(horizontalAlignment = Alignment.CenterHorizontally, modifier = Modifier.fillMaxWidth()) {
                Text("Pairing with ${p.peer} (${p.addr}).")
                Spacer(Modifier.height(16.dp))
                Text(p.code, fontSize = 36.sp, fontFamily = FontFamily.Monospace, fontWeight = FontWeight.Bold)
                Spacer(Modifier.height(16.dp))
                Text(
                    "${p.peer} shows a code too. If it is the same, confirm here and allow it there.",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        },
        confirmButton = { Button(onClick = { reply(true) }) { Text("The codes match") } },
        dismissButton = { TextButton(onClick = { reply(false) }) { Text("Cancel") } },
    )
}

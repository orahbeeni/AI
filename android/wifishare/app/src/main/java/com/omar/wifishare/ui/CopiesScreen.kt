package com.omar.wifishare.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Download
import androidx.compose.material.icons.filled.Upload
import androidx.compose.material3.Card
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.omar.wifishare.Copy
import com.omar.wifishare.Engine
import com.omar.wifishare.EngineState
import com.omar.wifishare.Native
import com.omar.wifishare.human
import kotlinx.coroutines.launch

/** Inbox uploads are named after the computer's inbox share. */
private const val INBOX = "@inbox"

fun describe(c: Copy): String =
    if (c.download) "from ${c.peer} to ${c.place.substringAfterLast('/')}"
    else if (c.place == INBOX) "to ${c.peer}'s Downloads"
    else "to ${c.peer}: ${c.place}"

@Composable
fun CopiesScreen(st: EngineState, modifier: Modifier) {
    val scope = rememberCoroutineScope()
    if (st.copies.isEmpty()) {
        Empty(
            "No copies yet",
            "Files you save from the computer, send to it, or share with wifishare from another app show up here.",
            modifier,
        )
        return
    }
    Column(modifier.fillMaxSize()) {
        Row(Modifier.fillMaxWidth().padding(horizontal = 8.dp), horizontalArrangement = Arrangement.End) {
            TextButton(
                enabled = st.copies.any { it.finished },
                onClick = { scope.launch { Engine.act { Native.clearFinished() } } },
            ) { Text("Clear finished") }
        }
        LazyColumn(Modifier.weight(1f).padding(horizontal = 12.dp)) {
            items(st.copies.reversed(), key = { it.id }) { c ->
                Card(Modifier.fillMaxWidth().padding(vertical = 4.dp)) {
                    Column(Modifier.padding(12.dp)) {
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            Icon(if (c.download) Icons.Default.Download else Icons.Default.Upload, null)
                            Spacer(Modifier.width(10.dp))
                            Column(Modifier.weight(1f)) {
                                Text(c.what.trim('"'), maxLines = 1, overflow = TextOverflow.Ellipsis)
                                Text(describe(c), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                            }
                            if (!c.finished) {
                                IconButton(onClick = { scope.launch { Engine.act { Native.cancel(c.id) } } }) {
                                    Icon(Icons.Default.Close, "Cancel")
                                }
                            }
                        }
                        Spacer(Modifier.height(8.dp))
                        LinearProgressIndicator(progress = { c.fraction }, modifier = Modifier.fillMaxWidth())
                        Spacer(Modifier.height(6.dp))
                        val (line, color) = when (c.state) {
                            "running" -> "${human(c.done)} of ${human(c.total)}" to Color.Unspecified
                            "waiting" -> "Waiting for ${c.peer}: ${c.detail}" to Amber
                            "done" -> "Done · ${human(c.total)}" to Green
                            "failed" -> "Failed: ${c.detail}" to Red
                            else -> "Cancelled" to Color.Unspecified
                        }
                        Text(line, style = MaterialTheme.typography.bodySmall, color = color)
                    }
                }
            }
        }
    }
}

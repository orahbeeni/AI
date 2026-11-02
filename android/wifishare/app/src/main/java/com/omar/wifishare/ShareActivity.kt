package com.omar.wifishare

import android.content.Intent
import android.net.Uri
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.Computer
import androidx.compose.material3.Button
import androidx.compose.material3.Icon
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.omar.wifishare.ui.Green
import com.omar.wifishare.ui.Red
import com.omar.wifishare.ui.StatusDot
import com.omar.wifishare.ui.WifishareTheme
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import org.json.JSONObject

/** "Share → wifishare": pick a computer, and the items go to its Downloads folder. */
class ShareActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val (uris, text) = Outgoing.fromIntent(intent)
        val names = uris.map { Outgoing.displayName(this, it) } + listOfNotNull(text?.let { "Shared text.txt" })
        setContent {
            WifishareTheme {
                Surface(shape = RoundedCornerShape(24.dp), tonalElevation = 6.dp, modifier = Modifier.fillMaxWidth()) {
                    SendSheet(uris, text, names, onDone = ::finish)
                }
            }
        }
    }
}

@Composable
private fun SendSheet(uris: List<Uri>, text: String?, names: List<String>, onDone: () -> Unit) {
    val context = androidx.compose.ui.platform.LocalContext.current
    val st by Engine.state.collectAsState()
    val scope = rememberCoroutineScope()
    var job by remember { mutableStateOf<Long?>(null) }
    var to by remember { mutableStateOf<String?>(null) }
    var error by remember { mutableStateOf<String?>(null) }

    fun send(computer: String) {
        to = computer
        scope.launch {
            Engine.act { Native.follow(computer) }?.let {
                error = it
                return@launch
            }
            val files = Engine.io { Outgoing.stage(context, uris, text) }
            if (files.isEmpty()) {
                error = "Nothing could be read from what was shared"
                return@launch
            }
            val answer = JSONObject(Engine.io { Native.send(jsonList(files)) })
            if (answer.has("ok")) {
                job = answer.getLong("ok")
                LinkService.keepForCopies(context)
            } else {
                error = answer.optString("error")
            }
        }
    }

    Column(Modifier.padding(24.dp)) {
        Text(
            if (names.size == 1) "Send \"${names[0]}\"" else "Send ${names.size} items",
            style = MaterialTheme.typography.titleLarge,
            maxLines = 2,
            overflow = TextOverflow.Ellipsis,
        )
        Spacer(Modifier.height(4.dp))
        Text("to a computer's Downloads folder", color = MaterialTheme.colorScheme.onSurfaceVariant)
        Spacer(Modifier.height(16.dp))

        val copy = job?.let { id -> st.copies.firstOrNull { it.id == id } }
        when {
            names.isEmpty() -> Text("There is nothing to send in what was shared.")
            st.computers.isEmpty() -> {
                Text("Pair your computer in the wifishare app first.")
                Spacer(Modifier.height(12.dp))
                Button(onClick = {
                    context.startActivity(Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
                    onDone()
                }) { Text("Open wifishare") }
            }
            error != null -> Text(error!!, color = Red)
            to == null -> for (c in st.computers) {
                Row(
                    Modifier.fillMaxWidth().clickable { send(c.name) }.padding(vertical = 12.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Icon(Icons.Default.Computer, null)
                    Spacer(Modifier.width(14.dp))
                    Column(Modifier.weight(1f)) {
                        Text(c.name, fontWeight = FontWeight.SemiBold)
                        if (c.current) {
                            Row(verticalAlignment = Alignment.CenterVertically) {
                                StatusDot(st.link)
                                Spacer(Modifier.width(6.dp))
                                Text(
                                    if (st.link.connected) "Connected" else "Not reachable right now",
                                    style = MaterialTheme.typography.bodySmall,
                                )
                            }
                        }
                    }
                }
            }
            copy == null -> {
                Text("Getting ready for $to…")
                Spacer(Modifier.height(8.dp))
                LinearProgressIndicator(Modifier.fillMaxWidth())
            }
            else -> {
                LaunchedEffect(copy.state) {
                    if (copy.state == "done") {
                        delay(1200)
                        onDone()
                    }
                }
                when (copy.state) {
                    "done" -> Row(verticalAlignment = Alignment.CenterVertically) {
                        Icon(Icons.Default.CheckCircle, null, tint = Green)
                        Spacer(Modifier.width(8.dp))
                        Text("Sent to $to")
                    }
                    "failed" -> Text("Failed: ${copy.detail}", color = Red)
                    "waiting" -> Text("Waiting for $to to be reachable… It carries on by itself.")
                    else -> Text("Sending to $to · ${human(copy.done)} of ${human(copy.total)}")
                }
                Spacer(Modifier.height(8.dp))
                LinearProgressIndicator(progress = { copy.fraction }, modifier = Modifier.fillMaxWidth())
            }
        }
        Spacer(Modifier.height(12.dp))
        Row(Modifier.fillMaxWidth(), horizontalArrangement = androidx.compose.foundation.layout.Arrangement.End) {
            TextButton(onClick = onDone) {
                Text(if (job != null && copy?.finished != true) "Hide (keeps sending)" else if (to == null) "Cancel" else "Close")
            }
        }
    }
}

package com.omar.wifishare.ui

import android.content.Intent
import android.net.Uri
import android.provider.Settings
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Computer
import androidx.compose.material3.Button
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.unit.dp
import com.omar.wifishare.Engine
import com.omar.wifishare.EngineState
import com.omar.wifishare.LinkService
import com.omar.wifishare.Native
import kotlinx.coroutines.launch

@Composable
private fun Heading(text: String) {
    Spacer(Modifier.height(20.dp))
    Text(text, style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.primary)
    Spacer(Modifier.height(8.dp))
}

@Composable
private fun SwitchRow(title: String, detail: String, checked: Boolean, enabled: Boolean = true, onChange: (Boolean) -> Unit) {
    Row(Modifier.fillMaxWidth().padding(vertical = 6.dp), verticalAlignment = Alignment.CenterVertically) {
        Column(Modifier.weight(1f)) {
            Text(title)
            Text(detail, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        Spacer(Modifier.width(12.dp))
        Switch(checked = checked, onCheckedChange = onChange, enabled = enabled)
    }
}

@Composable
fun SettingsScreen(st: EngineState, modifier: Modifier, onPair: () -> Unit, tell: (String) -> Unit) {
    val context = LocalContext.current
    val focus = LocalFocusManager.current
    val scope = rememberCoroutineScope()
    var name by remember { mutableStateOf(Engine.phoneName) }
    var allowAdd by remember { mutableStateOf(Engine.allowAdd) }
    var background by remember { mutableStateOf(Engine.background) }
    val allFiles = Engine.hasAllFiles()

    Column(modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 20.dp)) {
        Heading("This phone")
        OutlinedTextField(
            value = name,
            onValueChange = { name = it },
            label = { Text("Name computers see") },
            singleLine = true,
            modifier = Modifier.fillMaxWidth(),
            keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done),
            keyboardActions = KeyboardActions(onDone = {
                Engine.phoneName = name
                focus.clearFocus()
                tell("Saved. Computers see the new name after the next pairing.")
            }),
        )

        Heading("What computers can see")
        if (allFiles) {
            Text("Paired computers can browse all of this phone's storage (as \"Phone\") and copy from it.")
        } else {
            Text(
                "To let your computer browse this phone, and to save copies in Download, " +
                    "wifishare needs access to all files.",
            )
            Spacer(Modifier.height(8.dp))
            Button(onClick = {
                context.startActivity(
                    Intent(Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION, Uri.parse("package:${context.packageName}")),
                )
            }) { Text("Allow access to all files") }
        }
        SwitchRow(
            "Let computers add files",
            "They can copy files and folders onto this phone, never overwrite or delete.",
            allowAdd,
            enabled = allFiles,
        ) {
            allowAdd = it
            Engine.allowAdd = it
        }
        SwitchRow(
            "Stay linked in the background",
            "Keeps the link open with a notification, so the computer can browse this phone while wifishare is closed.",
            background,
        ) {
            background = it
            Engine.background = it
            LinkService.sync(context)
        }

        Heading("Computers")
        if (st.computers.isEmpty()) Text("None paired yet.", color = MaterialTheme.colorScheme.onSurfaceVariant)
        for (c in st.computers) {
            Row(Modifier.fillMaxWidth().padding(vertical = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                Icon(Icons.Default.Computer, null)
                Spacer(Modifier.width(12.dp))
                Column(Modifier.weight(1f)) {
                    Text(c.name, fontWeight = if (c.current) FontWeight.SemiBold else FontWeight.Normal)
                    Text(
                        listOfNotNull(if (c.current) "In use" else null, c.lastAddr?.substringBefore(':')).joinToString(" · "),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                TextButton(onClick = { scope.launch { Engine.act { Native.forget(c.name) }?.let(tell) } }) { Text("Forget") }
            }
        }
        Spacer(Modifier.height(8.dp))
        OutlinedButton(onClick = onPair) { Text("Pair a computer") }

        Spacer(Modifier.height(24.dp))
        HorizontalDivider()
        Spacer(Modifier.height(12.dp))
        Text(
            "Files saved from a computer go to ${Engine.downloads().path}. Anything you share with " +
                "wifishare from another app goes to the computer's Downloads folder.",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.height(24.dp))
    }
}

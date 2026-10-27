package com.omar.smsmanager.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.Send
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.compose.foundation.text.KeyboardOptions
import com.omar.smsmanager.data.ContactHit
import com.omar.smsmanager.data.Contacts
import com.omar.smsmanager.sms.SmsSender
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ComposeScreen(initialAddress: String, initialBody: String, onBack: () -> Unit, onSent: (String) -> Unit) {
    val ctx = LocalContext.current
    val scope = rememberCoroutineScope()
    var to by remember { mutableStateOf(initialAddress) }
    var body by remember { mutableStateOf(initialBody) }
    var picked by remember { mutableStateOf(initialAddress.isNotEmpty()) }

    val suggestions by produceState(emptyList<ContactHit>(), to, picked) {
        value = if (picked) emptyList() else withContext(Dispatchers.IO) { Contacts.search(ctx, to.trim()) }
    }

    fun send() {
        val address = to.trim()
        if (address.isEmpty() || body.isBlank()) return
        if (!hasSendPermission(ctx)) return toast(ctx, "Grant the SMS permission first (Messages tab)")
        scope.launch {
            withContext(Dispatchers.IO) { SmsSender.send(ctx, address, body) }
            onSent(address)
        }
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("New message") },
                navigationIcon = { IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, "Back") } },
                actions = {
                    IconButton(onClick = ::send, enabled = to.isNotBlank() && body.isNotBlank()) {
                        Icon(Icons.AutoMirrored.Filled.Send, "Send")
                    }
                },
            )
        },
    ) { pad ->
        Column(Modifier.fillMaxSize().padding(pad).imePadding().padding(16.dp)) {
            OutlinedTextField(
                value = to,
                onValueChange = { to = it; picked = false },
                label = { Text("To (name or number)") },
                singleLine = true,
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Text),
                modifier = Modifier.fillMaxWidth(),
            )
            suggestions.forEach { s ->
                ListItem(
                    modifier = Modifier.clickable { to = s.number; picked = true },
                    headlineContent = { Text(s.name) },
                    supportingContent = { Text(s.number) },
                )
            }
            OutlinedTextField(
                value = body,
                onValueChange = { body = it },
                label = { Text("Message") },
                modifier = Modifier.fillMaxWidth().padding(top = 12.dp),
                minLines = 3,
            )
        }
    }
}

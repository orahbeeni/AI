package com.omar.smsmanager.ui

import android.Manifest
import android.os.Build
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Block
import androidx.compose.material.icons.filled.Chat
import androidx.compose.material.icons.filled.FilterAlt
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material3.Badge
import androidx.compose.material3.BadgedBox
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import com.omar.smsmanager.data.ConversationCache
import com.omar.smsmanager.data.Contacts
import com.omar.smsmanager.data.Db
import com.omar.smsmanager.data.Ticker
import com.omar.smsmanager.sms.DefaultApp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext

private val TABS = listOf("Messages", "Blocked", "Rules", "Settings")

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun HomeScreen(onOpenThread: (Long, String) -> Unit, onCompose: () -> Unit) {
    val ctx = LocalContext.current
    val tick by Ticker.tick.collectAsState()
    var tab by rememberSaveable { mutableIntStateOf(0) }

    LaunchedEffect(tick) {
        delay(100) // let a burst of provider changes settle into one reload
        ConversationCache.refresh(ctx)
    }

    val blockedCount by produceState(0, tick) { value = withContext(Dispatchers.IO) { Db.get(ctx).blockedCount() } }

    Scaffold(
        topBar = { TopAppBar(title = { Text(TABS[tab]) }) },
        bottomBar = {
            NavigationBar {
                TABS.forEachIndexed { i, label ->
                    NavigationBarItem(
                        selected = tab == i,
                        onClick = { tab = i },
                        label = { Text(label) },
                        icon = {
                            val icon = when (i) {
                                0 -> Icons.Filled.Chat
                                1 -> Icons.Filled.Block
                                2 -> Icons.Filled.FilterAlt
                                else -> Icons.Filled.Settings
                            }
                            if (i == 1 && blockedCount > 0) {
                                BadgedBox(badge = { Badge { Text(blockedCount.toString()) } }) { Icon(icon, label) }
                            } else {
                                Icon(icon, label)
                            }
                        },
                    )
                }
            }
        },
    ) { pad ->
        Column(Modifier.padding(pad)) {
            SetupCard(tick)
            when (tab) {
                0 -> ConversationsScreen(onOpenThread, onCompose)
                1 -> BlockedScreen()
                2 -> RulesScreen()
                else -> SettingsScreen()
            }
        }
    }
}

private fun requiredPermissions(): Array<String> = buildList {
    add(Manifest.permission.READ_SMS)
    add(Manifest.permission.SEND_SMS)
    add(Manifest.permission.RECEIVE_SMS)
    add(Manifest.permission.READ_CONTACTS)
    if (Build.VERSION.SDK_INT >= 33) add(Manifest.permission.POST_NOTIFICATIONS)
}.toTypedArray()

/** Shown until we are the default SMS app and hold every permission. */
@Composable
private fun SetupCard(tick: Int) {
    val ctx = LocalContext.current
    val isDefault = remember(tick) { DefaultApp.isDefault(ctx) }
    val missing = remember(tick) {
        requiredPermissions().any { ctx.checkSelfPermission(it) != android.content.pm.PackageManager.PERMISSION_GRANTED }
    }

    val roleLauncher = rememberLauncherForActivityResult(ActivityResultContracts.StartActivityForResult()) { Ticker.bump() }
    val permLauncher = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) {
        Contacts.clearCache()
        Ticker.bump()
    }

    if (isDefault && !missing) return
    Card(
        Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp),
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.tertiaryContainer),
    ) {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            if (!isDefault) {
                Text("Spam blocking only works while SmsManager is your default SMS app.", style = MaterialTheme.typography.bodyMedium)
                Button(onClick = { DefaultApp.requestIntent(ctx)?.let(roleLauncher::launch) }) { Text("Make default SMS app") }
            }
            if (missing) {
                Text("Permissions are needed to read, send and receive messages and show names.", style = MaterialTheme.typography.bodyMedium)
                Row { Button(onClick = { permLauncher.launch(requiredPermissions()) }) { Text("Grant permissions") } }
            }
        }
    }
}

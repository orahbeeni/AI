package com.omar.smsmanager.ui

import android.content.Intent
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.FilterChip
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import com.omar.smsmanager.BuildConfig
import com.omar.smsmanager.data.AutoBackup
import com.omar.smsmanager.data.Backup
import com.omar.smsmanager.data.Settings
import com.omar.smsmanager.data.Ticker
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

@Composable
fun SettingsScreen() {
    val ctx = LocalContext.current
    val settings = remember { Settings(ctx) }
    var contactsAllowed by remember { mutableStateOf(settings.contactsAlwaysAllowed) }
    var retention by remember { mutableStateOf(settings.retentionDays) }
    var offerOtp by remember { mutableStateOf(settings.offerOtpCopy) }
    val tick by Ticker.tick.collectAsState() // refreshes the backup status line
    val scope = rememberCoroutineScope()

    val persistFlags = Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION or
        Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION
    fun keepAccess(uri: android.net.Uri): Boolean = try {
        ctx.contentResolver.takePersistableUriPermission(uri, Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION)
        true
    } catch (_: Exception) {
        false
    }

    val chooseFile = rememberLauncherForActivityResult(ActivityResultContracts.StartActivityForResult()) { r ->
        val uri = r.data?.data ?: return@rememberLauncherForActivityResult
        if (!keepAccess(uri)) return@rememberLauncherForActivityResult toast(ctx, "Couldn't get write access to that file")
        AutoBackup.setTarget(ctx, uri)
        scope.launch {
            val ok = withContext(Dispatchers.IO) { AutoBackup.writeIfChanged(ctx, force = true) }
            toast(ctx, if (ok) "Backup saved. It will update automatically." else "Couldn't write the file")
        }
    }
    val restoreFile = rememberLauncherForActivityResult(ActivityResultContracts.StartActivityForResult()) { r ->
        val uri = r.data?.data ?: return@rememberLauncherForActivityResult
        scope.launch {
            val res = withContext(Dispatchers.IO) { Backup.readText(ctx, uri)?.let { Backup.restore(ctx, it) } }
            if (res == null) return@launch toast(ctx, "That isn't an SmsManager backup file")
            contactsAllowed = settings.contactsAlwaysAllowed; retention = settings.retentionDays; offerOtp = settings.offerOtpCopy
            // Keep saving to the same file from now on, if the provider lets us write to it.
            val resumed = keepAccess(uri).also { if (it) AutoBackup.setTarget(ctx, uri) }
            toast(ctx, "Restored ${res.rulesAdded} rule(s)" + (if (res.rulesAlreadyThere > 0) ", ${res.rulesAlreadyThere} already there" else "") + if (resumed) ". Automatic backup resumed." else "")
        }
    }

    Column(Modifier.verticalScroll(rememberScrollState()).padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Section("Blocking")
        ToggleRow("Never block my contacts", "Senders in your address book skip the block rules.", contactsAllowed) {
            contactsAllowed = it; settings.contactsAlwaysAllowed = it
        }
        Text("Keep blocked messages for", style = MaterialTheme.typography.bodyMedium)
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            listOf(7 to "7 days", 30 to "30 days", 90 to "90 days", 0 to "Forever").forEach { (days, label) ->
                FilterChip(retention == days, { retention = days; settings.retentionDays = days }, { Text(label) })
            }
        }

        HorizontalDivider()
        Section("Notifications")
        ToggleRow("Copy button for one-time codes", "When a message contains a code, the notification shows a button like \"Copy 482913\".", offerOtp) {
            offerOtp = it; settings.offerOtpCopy = it
        }

        HorizontalDivider()
        Section("Backup")
        val backupUri = remember(tick) { AutoBackup.target(ctx) }
        val fileName = remember(backupUri) { backupUri?.let { Backup.displayName(ctx, it) } }
        Text(
            "Saves your rules and settings to a file you choose and keeps it up to date. If you ever uninstall the app, " +
                "restore from that file. (Installing a new version over the old one keeps everything without this.) " +
                "Blocked messages aren't included.",
            style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        val last = remember(tick) { settings.backupLast }
        val error = remember(tick) { settings.backupError }
        Text(
            when {
                backupUri == null -> "Automatic backup: not set up"
                error.isNotEmpty() -> "Automatic backup to ${fileName ?: "file"} is failing: $error"
                last > 0 -> "Automatic backup: ${fileName ?: "file"}, saved ${formatFull(ctx, last)}"
                else -> "Automatic backup: ${fileName ?: "file"}"
            },
            style = MaterialTheme.typography.bodyMedium,
            color = if (error.isNotEmpty() && backupUri != null) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurface,
        )
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            OutlinedButton(onClick = {
                chooseFile.launch(
                    Intent(Intent.ACTION_CREATE_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE).setType("application/json")
                        .putExtra(Intent.EXTRA_TITLE, "smsmanager-backup.json").addFlags(persistFlags)
                )
            }) { Text(if (backupUri == null) "Choose file…" else "Change file…") }
            OutlinedButton(onClick = {
                restoreFile.launch(
                    Intent(Intent.ACTION_OPEN_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE).setType("*/*").addFlags(persistFlags)
                )
            }) { Text("Restore…") }
        }
        if (backupUri != null) {
            OutlinedButton(onClick = {
                scope.launch {
                    val ok = withContext(Dispatchers.IO) { AutoBackup.writeIfChanged(ctx, force = true) }
                    toast(ctx, if (ok) "Backup saved" else "Couldn't write the file")
                }
            }) { Text("Back up now") }
        }

        HorizontalDivider()
        Section("About")
        Text("SmsManager ${BuildConfig.VERSION_NAME} (build ${BuildConfig.VERSION_CODE})", style = MaterialTheme.typography.bodyMedium)
        Text(
            "SMS only for now. Picture and group (MMS) messages aren't downloaded.",
            style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

@Composable
private fun Section(title: String) = Text(title, style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.primary)

@Composable
private fun ToggleRow(title: String, subtitle: String, checked: Boolean, onChange: (Boolean) -> Unit) {
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        Column(Modifier.weight(1f)) {
            Text(title, style = MaterialTheme.typography.bodyLarge)
            Text(subtitle, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        Switch(checked, onChange)
    }
}

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
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.FilterChip
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
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
    var offerKeep by remember { mutableStateOf<android.net.Uri?>(null) } // a just-imported file we may keep updating
    val tick by Ticker.tick.collectAsState() // refreshes the backup status line
    val scope = rememberCoroutineScope()

    val persistFlags = Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION or
        Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION
    val rw = Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION
    fun keepAccess(uri: android.net.Uri): Boolean = try {
        ctx.contentResolver.takePersistableUriPermission(uri, rw)
        true
    } catch (_: Exception) {
        false
    }

    /** Makes [uri] the file that is kept up to date, and writes it now. */
    fun startKeepingUpdated(uri: android.net.Uri) {
        if (!keepAccess(uri)) return toast(ctx, "Couldn't get write access to that file")
        AutoBackup.setTarget(ctx, uri)
        scope.launch {
            val ok = withContext(Dispatchers.IO) { AutoBackup.writeIfChanged(ctx, force = true) }
            toast(ctx, if (ok) "Backup saved. It will update automatically." else "Couldn't write the file")
        }
    }

    val pickBackupFile = rememberLauncherForActivityResult(ActivityResultContracts.StartActivityForResult()) { r ->
        r.data?.data?.let(::startKeepingUpdated)
    }
    val importFile = rememberLauncherForActivityResult(ActivityResultContracts.StartActivityForResult()) { r ->
        val uri = r.data?.data ?: return@rememberLauncherForActivityResult
        scope.launch {
            val res = withContext(Dispatchers.IO) { Backup.readText(ctx, uri)?.let { Backup.restore(ctx, it) } }
            if (res == null) return@launch toast(ctx, "That isn't an SmsManager settings file")
            contactsAllowed = settings.contactsAlwaysAllowed; retention = settings.retentionDays; offerOtp = settings.offerOtpCopy
            toast(
                ctx,
                "Imported ${res.rulesAdded} rule(s)" + if (res.rulesAlreadyThere > 0) ", ${res.rulesAlreadyThere} already there" else "",
            )
            // After a reinstall this is the moment to carry on with the same file.
            if (AutoBackup.target(ctx) == null) offerKeep = uri
        }
    }
    val exportFile = rememberLauncherForActivityResult(ActivityResultContracts.StartActivityForResult()) { r ->
        val uri = r.data?.data ?: return@rememberLauncherForActivityResult
        scope.launch {
            val n = withContext(Dispatchers.IO) { Backup.exportTo(ctx, uri) }
            toast(ctx, if (n != null) "Exported $n rule(s) and your settings" else "Couldn't write the file")
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
        Text(
            "Export saves your rules and settings to a file, for example to move to another phone. Import loads one: " +
                "rules you don't already have are added and your settings are replaced. Blocked messages aren't included. " +
                "After uninstalling and reinstalling, use Import with your backup file. " +
                "(Installing a new version over the old one keeps everything without any of this.)",
            style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            OutlinedButton(onClick = {
                exportFile.launch(
                    Intent(Intent.ACTION_CREATE_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE).setType("application/json")
                        .putExtra(Intent.EXTRA_TITLE, "smsmanager-settings.json")
                )
            }) { Text("Export…") }
            OutlinedButton(onClick = {
                importFile.launch(
                    Intent(Intent.ACTION_OPEN_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE).setType("*/*").addFlags(persistFlags)
                )
            }) { Text("Import…") }
        }

        val backupUri = remember(tick) { AutoBackup.target(ctx) }
        val fileName = remember(backupUri) { backupUri?.let { Backup.displayName(ctx, it) } }
        val last = remember(tick) { settings.backupLast }
        val error = remember(tick) { settings.backupError }
        val failing = backupUri != null && error.isNotEmpty()
        ToggleRow(
            "Keep a backup file up to date",
            when {
                backupUri == null -> "Saves to a file you choose after every change, so an uninstall doesn't cost you your rules."
                failing -> "Can't write to ${fileName ?: "the file"}: $error. Switch off and on to pick another file."
                last > 0 -> "Saving to ${fileName ?: "the file"}. Last saved ${formatFull(ctx, last)}."
                else -> "Saving to ${fileName ?: "the file"}."
            },
            checked = backupUri != null,
            subtitleColor = if (failing) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant,
        ) { on ->
            if (on) {
                pickBackupFile.launch(
                    Intent(Intent.ACTION_CREATE_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE).setType("application/json")
                        .putExtra(Intent.EXTRA_TITLE, "smsmanager-backup.json").addFlags(persistFlags)
                )
            } else {
                backupUri?.let { try { ctx.contentResolver.releasePersistableUriPermission(it, rw) } catch (_: Exception) {} }
                AutoBackup.setTarget(ctx, null)
                Ticker.bump()
            }
        }

        HorizontalDivider()
        Section("About")
        Text("SmsManager ${BuildConfig.VERSION_NAME} (build ${BuildConfig.VERSION_CODE})", style = MaterialTheme.typography.bodyMedium)
        Text(
            "SMS only for now. Picture and group (MMS) messages aren't downloaded.",
            style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }

    offerKeep?.let { uri ->
        AlertDialog(
            onDismissRequest = { offerKeep = null },
            title = { Text("Keep this file up to date?") },
            text = { Text("From now on SmsManager can save your rules and settings back to this file after every change. That's the usual choice after a reinstall.") },
            confirmButton = { TextButton(onClick = { offerKeep = null; startKeepingUpdated(uri) }) { Text("Yes, keep it updated") } },
            dismissButton = { TextButton(onClick = { offerKeep = null }) { Text("No thanks") } },
        )
    }
}

@Composable
private fun Section(title: String) = Text(title, style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.primary)

@Composable
private fun ToggleRow(
    title: String,
    subtitle: String,
    checked: Boolean,
    subtitleColor: Color = MaterialTheme.colorScheme.onSurfaceVariant,
    onChange: (Boolean) -> Unit,
) {
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        Column(Modifier.weight(1f)) {
            Text(title, style = MaterialTheme.typography.bodyLarge)
            Text(subtitle, style = MaterialTheme.typography.bodySmall, color = subtitleColor)
        }
        Switch(checked, onChange)
    }
}

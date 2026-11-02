package com.omar.wifishare.ui

import android.text.format.DateUtils
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.InsertDriveFile
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.ChevronRight
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.CreateNewFolder
import androidx.compose.material.icons.filled.Download
import androidx.compose.material.icons.filled.Folder
import androidx.compose.material.icons.filled.FolderShared
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Upload
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.BottomAppBar
import androidx.compose.material3.ExtendedFloatingActionButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.omar.wifishare.Answer
import com.omar.wifishare.Engine
import com.omar.wifishare.EngineState
import com.omar.wifishare.Entry
import com.omar.wifishare.LinkService
import com.omar.wifishare.Native
import com.omar.wifishare.Outgoing
import com.omar.wifishare.ShareInfo
import com.omar.wifishare.answer
import com.omar.wifishare.human
import com.omar.wifishare.jsonList
import com.omar.wifishare.parseEntries
import com.omar.wifishare.parseShares
import kotlinx.coroutines.launch

/** A row: a shared folder (at the top level), a folder or a file. */
private data class Row(val name: String, val dir: Boolean, val size: Long, val modified: Long)

/** The computer's shared folders: open folders, select files, save them here or send files there. */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun BrowseScreen(st: EngineState, modifier: Modifier, onPair: () -> Unit, tell: (String) -> Unit, showCopies: () -> Unit) {
    val link = st.link
    if (st.computers.isEmpty()) {
        Empty(
            "Pair your computer",
            "Open wifishare on your Mac or Ubuntu machine, start sharing, then pair it here. " +
                "You can then browse the folders it shares and send it files from any app's Share menu.",
            modifier,
            "Pair a computer" to onPair,
        )
        return
    }
    if (!link.connected) {
        Empty(
            "Waiting for ${link.computer ?: st.current?.name ?: "the computer"}",
            "It connects by itself as soon as the computer is on this Wi-Fi with wifishare sharing." +
                (link.error?.let { "\n\nLast attempt: $it" } ?: ""),
            modifier,
        )
        return
    }

    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var share by remember(link.computer) { mutableStateOf<String?>(null) }
    var path by remember(link.computer) { mutableStateOf("") }
    var shares by remember(link.computer) { mutableStateOf<List<ShareInfo>>(emptyList()) }
    var rows by remember(link.computer) { mutableStateOf<List<Row>>(emptyList()) }
    var loading by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    val selected = remember(link.computer, share, path) { mutableStateListOf<String>() }
    var reload by remember { mutableStateOf(0) }
    var newFolder by remember { mutableStateOf<String?>(null) }
    val writable = share != null && shares.firstOrNull { it.name == share }?.write == true

    LaunchedEffect(link.computer, link.connected, share, path, reload) {
        loading = true
        error = null
        val s = share
        if (s == null) {
            when (val a = Engine.io { answer(Native.shares(), ::parseShares) }) {
                is Answer.Ok -> {
                    shares = a.value
                    rows = a.value.map { Row(it.name, true, 0, 0) }
                }
                is Answer.Failed -> error = a.error
            }
        } else {
            when (val a = Engine.io { answer(Native.list(s, path), ::parseEntries) }) {
                is Answer.Ok -> rows = a.value.filter { !it.name.startsWith(".") }.map { Row(it.name, it.dir, it.size, it.modified) }
                is Answer.Failed -> error = a.error
            }
        }
        loading = false
    }

    fun open(name: String) {
        if (share == null) {
            share = name
            path = ""
        } else {
            path = if (path.isEmpty()) name else "$path/$name"
        }
    }

    fun up() {
        when {
            share == null -> {}
            path.isEmpty() -> share = null
            else -> path = path.substringBeforeLast('/', "")
        }
    }
    BackHandler(enabled = share != null || selected.isNotEmpty()) {
        if (selected.isNotEmpty()) selected.clear() else up()
    }

    val pick = rememberLauncherForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { uris ->
        val s = share ?: return@rememberLauncherForActivityResult
        if (uris.isEmpty()) return@rememberLauncherForActivityResult
        val dir = path
        scope.launch {
            val files = Engine.io { Outgoing.stage(context, uris, null) }
            Engine.act { Native.upload(s, dir, jsonList(files)) }?.let(tell) ?: run {
                LinkService.keepForCopies(context)
                tell("Sending ${files.size} ${if (files.size == 1) "file" else "files"} to ${link.computer}")
            }
        }
    }

    Box(modifier.fillMaxSize()) {
        Column(Modifier.fillMaxSize()) {
            Breadcrumbs(share, path, onRoot = { share = null; path = "" }, onShare = { path = "" }, onPart = { path = it }) {
                IconButton(onClick = { reload++ }) { Icon(Icons.Default.Refresh, "Refresh") }
                if (writable) IconButton(onClick = { newFolder = "" }) { Icon(Icons.Default.CreateNewFolder, "New folder") }
            }
            if (loading) LinearProgressIndicator(Modifier.fillMaxWidth()) else Spacer(Modifier.padding(top = 4.dp))
            when {
                error != null -> Empty("Could not open this", error!!, action = "Try again" to { reload++ })
                !loading && rows.isEmpty() && share == null -> Empty(
                    "Nothing shared yet",
                    "On ${link.computer}, open wifishare's Files tab and choose folders under \"Folders I share\". " +
                        "Files you send from the Share menu still arrive in its Downloads.",
                )
                !loading && rows.isEmpty() -> Empty("Empty folder", "")
                else -> LazyColumn(Modifier.weight(1f)) {
                    items(rows, key = { it.name }) { r ->
                        val isSel = r.name in selected
                        Row(
                            Modifier
                                .fillMaxWidth()
                                .background(if (isSel) MaterialTheme.colorScheme.secondaryContainer else MaterialTheme.colorScheme.surface)
                                .combinedClickable(
                                    onClick = {
                                        when {
                                            selected.isNotEmpty() && share != null -> if (isSel) selected.remove(r.name) else selected.add(r.name)
                                            r.dir -> open(r.name)
                                            else -> selected.add(r.name)
                                        }
                                    },
                                    onLongClick = { if (share != null && !isSel) selected.add(r.name) },
                                )
                                .padding(horizontal = 16.dp, vertical = 12.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Icon(
                                when {
                                    isSel -> Icons.Default.CheckCircle
                                    share == null -> Icons.Default.FolderShared
                                    r.dir -> Icons.Default.Folder
                                    else -> Icons.AutoMirrored.Filled.InsertDriveFile
                                },
                                null,
                                tint = if (r.dir || isSel) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                            Spacer(Modifier.width(16.dp))
                            Column(Modifier.weight(1f)) {
                                Text(r.name, maxLines = 1, overflow = TextOverflow.Ellipsis)
                                val detail = when {
                                    share == null -> if (shares.firstOrNull { it.name == r.name }?.write == true) "You may add files" else "Read-only"
                                    r.dir -> ""
                                    else -> listOf(
                                        human(r.size),
                                        if (r.modified > 0) DateUtils.getRelativeTimeSpanString(r.modified).toString() else "",
                                    ).filter { it.isNotEmpty() }.joinToString(" · ")
                                }
                                if (detail.isNotEmpty()) {
                                    Text(detail, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                                }
                            }
                            if (r.dir && selected.isEmpty()) Icon(Icons.Default.ChevronRight, null, tint = MaterialTheme.colorScheme.onSurfaceVariant)
                        }
                    }
                }
            }
            if (selected.isNotEmpty()) {
                BottomAppBar {
                    IconButton(onClick = { selected.clear() }) { Icon(Icons.Default.Close, "Clear selection") }
                    Text("${selected.size} selected", Modifier.weight(1f))
                    TextButton(onClick = {
                        val (s, dir, names) = Triple(share!!, path, selected.toList())
                        selected.clear()
                        scope.launch {
                            val dest = Engine.downloads()
                            Engine.act { Native.download(s, dir, jsonList(names), dest.path) }?.let(tell) ?: run {
                                LinkService.keepForCopies(context)
                                tell("Copying to ${dest.name}")
                                showCopies()
                            }
                        }
                    }) {
                        Icon(Icons.Default.Download, null)
                        Spacer(Modifier.width(6.dp))
                        Text("Save to phone")
                    }
                }
            }
        }
        if (writable && selected.isEmpty()) {
            ExtendedFloatingActionButton(
                onClick = { pick.launch(arrayOf("*/*")) },
                icon = { Icon(Icons.Default.Upload, null) },
                text = { Text("Send files here") },
                modifier = Modifier.align(Alignment.BottomEnd).padding(16.dp),
            )
        }
    }

    newFolder?.let { name ->
        AlertDialog(
            onDismissRequest = { newFolder = null },
            title = { Text("New folder") },
            text = { OutlinedTextField(value = name, onValueChange = { newFolder = it }, singleLine = true, label = { Text("Name") }) },
            confirmButton = {
                TextButton(enabled = name.isNotBlank(), onClick = {
                    val (s, target) = share!! to (if (path.isEmpty()) name.trim() else "$path/${name.trim()}")
                    newFolder = null
                    scope.launch {
                        Engine.act { Native.mkdir(s, target) }?.let(tell)
                        reload++
                    }
                }) { Text("Create") }
            },
            dismissButton = { TextButton(onClick = { newFolder = null }) { Text("Cancel") } },
        )
    }
}

@Composable
private fun Breadcrumbs(
    share: String?,
    path: String,
    onRoot: () -> Unit,
    onShare: () -> Unit,
    onPart: (String) -> Unit,
    actions: @Composable () -> Unit,
) {
    Row(Modifier.fillMaxWidth().padding(start = 8.dp), verticalAlignment = Alignment.CenterVertically) {
        Row(Modifier.weight(1f).horizontalScroll(rememberScrollState()), verticalAlignment = Alignment.CenterVertically) {
            TextButton(onClick = onRoot) { Text("Shared folders") }
            if (share != null) {
                Icon(Icons.Default.ChevronRight, null, tint = MaterialTheme.colorScheme.onSurfaceVariant)
                TextButton(onClick = onShare) { Text(share) }
            }
            val parts = if (path.isEmpty()) emptyList() else path.split('/')
            parts.forEachIndexed { i, part ->
                Icon(Icons.Default.ChevronRight, null, tint = MaterialTheme.colorScheme.onSurfaceVariant)
                TextButton(onClick = { onPart(parts.take(i + 1).joinToString("/")) }) { Text(part) }
            }
        }
        actions()
    }
}

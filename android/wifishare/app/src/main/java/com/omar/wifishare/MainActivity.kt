package com.omar.wifishare

import android.Manifest
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.ArrowDropDown
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Computer
import androidx.compose.material.icons.filled.Folder
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material.icons.filled.SwapVert
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Badge
import androidx.compose.material3.BadgedBox
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.omar.wifishare.ui.BrowseScreen
import com.omar.wifishare.ui.CodeDialog
import com.omar.wifishare.ui.CopiesScreen
import com.omar.wifishare.ui.PairDialog
import com.omar.wifishare.ui.SettingsScreen
import com.omar.wifishare.ui.StatusDot
import com.omar.wifishare.ui.WifishareTheme
import com.omar.wifishare.ui.linkText
import kotlinx.coroutines.launch

class MainActivity : ComponentActivity() {
    private val askNotifications = registerForActivityResult(ActivityResultContracts.RequestPermission()) {}

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        if (Build.VERSION.SDK_INT >= 33) askNotifications.launch(Manifest.permission.POST_NOTIFICATIONS)
        setContent { WifishareTheme { Main() } }
    }

    override fun onResume() {
        super.onResume()
        // The storage permission may have just been granted in Settings.
        Engine.apply()
        LinkService.sync(this)
    }
}

private enum class Tab(val label: String) { Browse("Computer"), Copies("Copies"), Settings("Settings") }

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun Main() {
    val st by Engine.state.collectAsState()
    var tab by rememberSaveable { mutableStateOf(Tab.Browse) }
    var pairing by rememberSaveable { mutableStateOf(false) }
    val snack = remember { SnackbarHostState() }
    val scope = rememberCoroutineScope()

    var lastNotice by remember { mutableStateOf<Long?>(null) }
    LaunchedEffect(st.notice) {
        val n = st.notice ?: return@LaunchedEffect
        if (n.id != lastNotice) {
            lastNotice = n.id
            if (n.text.startsWith("Paired")) pairing = false
            snack.showSnackbar(n.text)
        }
    }
    val tell: (String) -> Unit = { scope.launch { snack.showSnackbar(it) } }

    Scaffold(
        topBar = {
            TopAppBar(title = { ComputerSwitcher(st, onPair = { pairing = true }, tell = tell) })
        },
        bottomBar = {
            NavigationBar {
                for (t in Tab.entries) {
                    NavigationBarItem(
                        selected = tab == t,
                        onClick = { tab = t },
                        label = { Text(t.label) },
                        icon = {
                            val running = st.copies.count { !it.finished }
                            when (t) {
                                Tab.Browse -> Icon(Icons.Default.Folder, null)
                                Tab.Copies -> BadgedBox(badge = { if (running > 0) Badge { Text("$running") } }) {
                                    Icon(Icons.Default.SwapVert, null)
                                }
                                Tab.Settings -> Icon(Icons.Default.Settings, null)
                            }
                        },
                    )
                }
            }
        },
        snackbarHost = { SnackbarHost(snack) },
    ) { pad ->
        val modifier = Modifier.padding(pad)
        when (tab) {
            Tab.Browse -> BrowseScreen(st, modifier, onPair = { pairing = true }, tell = tell, showCopies = { tab = Tab.Copies })
            Tab.Copies -> CopiesScreen(st, modifier)
            Tab.Settings -> SettingsScreen(st, modifier, onPair = { pairing = true }, tell = tell)
        }
    }

    if (pairing) PairDialog(st, onClose = { pairing = false })
    st.prompts.firstOrNull()?.let { CodeDialog(it) }
}

/** The computer this phone works with, and a menu to switch to another or pair a new one. */
@Composable
private fun ComputerSwitcher(st: EngineState, onPair: () -> Unit, tell: (String) -> Unit) {
    var open by remember { mutableStateOf(false) }
    var confirm by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()
    fun switchTo(name: String) {
        scope.launch { Engine.act { Native.follow(name) }?.let(tell) }
    }
    Row(Modifier.clickable { open = true }, verticalAlignment = Alignment.CenterVertically) {
        Column(Modifier.weight(1f, fill = false)) {
            Text(
                st.current?.name ?: "No computer yet",
                style = MaterialTheme.typography.titleLarge,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Row(verticalAlignment = Alignment.CenterVertically) {
                StatusDot(st.link)
                Spacer(Modifier.width(6.dp))
                Text(
                    linkText(st.link),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
        Icon(Icons.Default.ArrowDropDown, "Switch computer")
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            for (c in st.computers) {
                DropdownMenuItem(
                    text = { Text(c.name) },
                    leadingIcon = { Icon(Icons.Default.Computer, null) },
                    trailingIcon = { if (c.current) Icon(Icons.Default.Check, "In use") },
                    onClick = {
                        open = false
                        if (!c.current) {
                            if (st.copies.any { !it.finished }) confirm = c.name else switchTo(c.name)
                        }
                    },
                )
            }
            if (st.computers.isNotEmpty()) HorizontalDivider()
            DropdownMenuItem(
                text = { Text("Pair a computer…") },
                leadingIcon = { Icon(Icons.Default.Add, null) },
                onClick = {
                    open = false
                    onPair()
                },
            )
        }
    }
    confirm?.let { name ->
        val running = st.copies.count { !it.finished }
        AlertDialog(
            onDismissRequest = { confirm = null },
            title = { Text("Switch to $name?") },
            text = { Text("$running ${if (running == 1) "copy is" else "copies are"} still running with ${st.current?.name}. Switching stops ${if (running == 1) "it" else "them"}.") },
            confirmButton = {
                TextButton(onClick = {
                    confirm = null
                    switchTo(name)
                }) { Text("Switch") }
            },
            dismissButton = { TextButton(onClick = { confirm = null }) { Text("Stay") } },
        )
    }
}

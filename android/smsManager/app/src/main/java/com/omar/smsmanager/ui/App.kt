package com.omar.smsmanager.ui

import androidx.activity.compose.BackHandler
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.MutableState
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.remember

sealed interface Route {
    data object Home : Route
    data class Thread(val threadId: Long, val address: String) : Route
    data class Compose(val address: String = "", val body: String = "") : Route
}

@Composable
fun App(pending: MutableState<Route?>) {
    val stack = remember { mutableStateListOf<Route>(Route.Home) }

    LaunchedEffect(pending.value) {
        pending.value?.let {
            // A notification tap for the thread already on screen shouldn't stack a second copy.
            if (stack.last() != it) stack.add(it)
            pending.value = null
        }
    }
    BackHandler(enabled = stack.size > 1) { stack.removeAt(stack.lastIndex) }

    when (val route = stack.last()) {
        Route.Home -> HomeScreen(
            onOpenThread = { id, address -> stack.add(Route.Thread(id, address)) },
            onCompose = { stack.add(Route.Compose()) },
        )
        is Route.Thread -> ThreadScreen(
            threadId = route.threadId,
            address = route.address,
            onBack = { stack.removeAt(stack.lastIndex) },
        )
        is Route.Compose -> ComposeScreen(
            initialAddress = route.address,
            initialBody = route.body,
            onBack = { stack.removeAt(stack.lastIndex) },
            onSent = { address ->
                stack.removeAt(stack.lastIndex)
                stack.add(Route.Thread(0, address))
            },
        )
    }
}

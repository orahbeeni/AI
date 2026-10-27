package com.omar.smsmanager.ui

import android.content.Context
import android.text.format.DateFormat
import android.text.format.DateUtils
import android.widget.Toast
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import java.util.Date

fun toast(ctx: Context, text: String) = Toast.makeText(ctx, text, Toast.LENGTH_SHORT).show()

/** Time for today's messages, short date otherwise. */
fun formatWhen(ctx: Context, millis: Long): String =
    if (DateUtils.isToday(millis)) DateFormat.getTimeFormat(ctx).format(Date(millis))
    else DateUtils.formatDateTime(ctx, millis, DateUtils.FORMAT_SHOW_DATE or DateUtils.FORMAT_ABBREV_MONTH)

fun formatFull(ctx: Context, millis: Long): String =
    DateUtils.formatDateTime(
        ctx, millis,
        DateUtils.FORMAT_SHOW_DATE or DateUtils.FORMAT_SHOW_TIME or DateUtils.FORMAT_ABBREV_MONTH,
    )

@Composable
fun Avatar(label: String, size: Dp = 40.dp) {
    val initial = label.firstOrNull { it.isLetterOrDigit() }?.uppercaseChar()?.toString() ?: "#"
    Box(
        Modifier.size(size).clip(CircleShape).background(MaterialTheme.colorScheme.primaryContainer),
        contentAlignment = Alignment.Center,
    ) {
        Text(initial, color = MaterialTheme.colorScheme.onPrimaryContainer, style = MaterialTheme.typography.titleMedium)
    }
}

@Composable
fun EmptyState(text: String, modifier: Modifier = Modifier) {
    Box(modifier.fillMaxSize().padding(32.dp), contentAlignment = Alignment.Center) {
        Text(text, textAlign = TextAlign.Center, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

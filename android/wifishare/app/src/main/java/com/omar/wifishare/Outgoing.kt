package com.omar.wifishare

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.provider.OpenableColumns
import androidx.core.content.IntentCompat
import java.io.File

/**
 * Files to send, copied out of other apps' content URIs into the cache: the engine sends paths,
 * and a shared URI may stop being readable once the sharing app moves on.
 */
object Outgoing {
    private fun root(context: Context) = File(context.cacheDir, "outgoing")

    /** The URIs (and text) an ACTION_SEND or ACTION_SEND_MULTIPLE intent carries. */
    fun fromIntent(intent: Intent): Pair<List<Uri>, String?> {
        val uris = when (intent.action) {
            Intent.ACTION_SEND_MULTIPLE ->
                IntentCompat.getParcelableArrayListExtra(intent, Intent.EXTRA_STREAM, Uri::class.java).orEmpty()
            else -> listOfNotNull(IntentCompat.getParcelableExtra(intent, Intent.EXTRA_STREAM, Uri::class.java))
        }
        val text = if (uris.isEmpty()) intent.getStringExtra(Intent.EXTRA_TEXT) else null
        return uris to text
    }

    fun displayName(context: Context, uri: Uri): String {
        context.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { c ->
            if (c.moveToFirst()) c.getString(0)?.let { return clean(it) }
        }
        return clean(uri.lastPathSegment?.substringAfterLast('/') ?: "file")
    }

    private fun clean(name: String) = name.replace(Regex("[/\\\\:\u0000]"), "_").ifBlank { "file" }

    /** Copies `uris` (and `text`, as a .txt file) into a fresh folder and returns the paths. */
    fun stage(context: Context, uris: List<Uri>, text: String?): List<String> {
        val dir = File(root(context), System.currentTimeMillis().toString()).apply { mkdirs() }
        val out = mutableListOf<String>()
        val taken = mutableSetOf<String>()
        fun free(name: String): File {
            var candidate = name
            var n = 2
            while (!taken.add(candidate)) {
                val dot = name.lastIndexOf('.').takeIf { it > 0 } ?: name.length
                candidate = "${name.substring(0, dot)} ($n)${name.substring(dot)}"
                n++
            }
            return File(dir, candidate)
        }
        for (uri in uris) {
            val file = free(displayName(context, uri))
            context.contentResolver.openInputStream(uri)?.use { input -> file.outputStream().use { input.copyTo(it) } }
                ?: continue
            out += file.path
        }
        if (text != null) {
            val file = free("Shared text.txt")
            file.writeText(text)
            out += file.path
        }
        return out
    }

    /** Forgets staged files from earlier days (a copy that old has finished or given up). */
    fun prune(context: Context) {
        val cutoff = System.currentTimeMillis() - 24 * 3600_000L
        root(context).listFiles()?.filter { it.lastModified() < cutoff }?.forEach { it.deleteRecursively() }
    }
}

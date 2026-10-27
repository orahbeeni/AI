package com.omar.smsmanager.sms

import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Context
import android.os.Build
import android.os.PersistableBundle

object ClipboardCopy {
    /** Copies a one-time code. Marked sensitive so Android 13+ hides it in the clipboard preview. */
    fun copyCode(ctx: Context, code: String) {
        val clip = ClipData.newPlainText("code", code)
        if (Build.VERSION.SDK_INT >= 33) {
            clip.description.extras = PersistableBundle().apply { putBoolean(ClipDescription.EXTRA_IS_SENSITIVE, true) }
        }
        ctx.getSystemService(ClipboardManager::class.java).setPrimaryClip(clip)
    }
}

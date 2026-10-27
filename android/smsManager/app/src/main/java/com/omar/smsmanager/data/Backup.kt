package com.omar.smsmanager.data

import android.content.Context
import android.net.Uri
import android.provider.OpenableColumns
import com.omar.smsmanager.spam.MatchMode
import com.omar.smsmanager.spam.Rule
import com.omar.smsmanager.spam.RuleAction
import com.omar.smsmanager.spam.RuleTarget
import org.json.JSONArray
import org.json.JSONException
import org.json.JSONObject

/**
 * Settings and rules as a small JSON file. Messages are deliberately not included: blocked texts are
 * private, and the backup file may live in cloud storage.
 */
object Backup {
    private const val APP = "smsManager"
    private const val VERSION = 1

    class RestoreResult(val rulesAdded: Int, val rulesAlreadyThere: Int)

    fun toJson(ctx: Context, exportedAt: Long? = null): String {
        val s = Settings(ctx)
        val rules = JSONArray()
        Db.get(ctx).rules().sortedBy { it.id }.forEach { r ->
            rules.put(
                JSONObject()
                    .put("action", r.action.name).put("target", r.target.name)
                    .put("pattern", r.pattern).put("mode", r.mode.name)
                    .put("enabled", r.enabled).put("hits", r.hits)
            )
        }
        return JSONObject().apply {
            put("app", APP)
            put("version", VERSION)
            exportedAt?.let { put("exportedAt", it) }
            put(
                "settings",
                JSONObject()
                    .put("contactsAlwaysAllowed", s.contactsAlwaysAllowed)
                    .put("retentionDays", s.retentionDays)
            )
            put("rules", rules)
        }.toString(2)
    }

    /** Rules are merged in (duplicates skipped); settings in the file replace the current ones. Null if [text] isn't our file. */
    fun restore(ctx: Context, text: String): RestoreResult? {
        val root = try { JSONObject(text) } catch (_: JSONException) { return null }
        if (root.optString("app") != APP) return null

        root.optJSONObject("settings")?.let { st ->
            val s = Settings(ctx)
            s.contactsAlwaysAllowed = st.optBoolean("contactsAlwaysAllowed", s.contactsAlwaysAllowed)
            s.retentionDays = st.optInt("retentionDays", s.retentionDays).coerceAtLeast(0)
        }

        val incoming = ArrayList<Rule>()
        val arr = root.optJSONArray("rules") ?: JSONArray()
        for (i in 0 until arr.length()) {
            val o = arr.optJSONObject(i) ?: continue
            try {
                incoming.add(
                    Rule(
                        action = RuleAction.valueOf(o.getString("action")),
                        target = RuleTarget.valueOf(o.getString("target")),
                        pattern = o.getString("pattern"),
                        mode = MatchMode.valueOf(o.optString("mode", "WILDCARD")),
                        enabled = o.optBoolean("enabled", true),
                        hits = o.optInt("hits", 0),
                    )
                )
            } catch (_: Exception) {
                // skip a damaged entry rather than failing the whole restore
            }
        }
        val added = Db.get(ctx).importRules(incoming)
        return RestoreResult(added, incoming.size - added)
    }

    fun readText(ctx: Context, uri: Uri): String? = try {
        ctx.contentResolver.openInputStream(uri)?.bufferedReader()?.use { it.readText() }
    } catch (_: Exception) {
        null
    }

    fun displayName(ctx: Context, uri: Uri): String? = try {
        ctx.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)
            ?.use { if (it.moveToFirst()) it.getString(0) else null }
    } catch (_: Exception) {
        null
    }
}

/** Keeps the chosen backup file current. Called from the Application, debounced, and skipped when nothing changed. */
object AutoBackup {
    fun target(ctx: Context): Uri? = Settings(ctx).backupUri.takeIf { it.isNotEmpty() }?.let(Uri::parse)

    fun setTarget(ctx: Context, uri: Uri?) {
        val s = Settings(ctx)
        s.backupUri = uri?.toString().orEmpty()
        s.backupHash = 0
        s.backupError = ""
        if (uri == null) s.backupLast = 0
    }

    /** Returns true when the file was written. */
    fun writeIfChanged(ctx: Context, force: Boolean = false): Boolean {
        val uri = target(ctx) ?: return false
        val s = Settings(ctx)
        val hash = Backup.toJson(ctx).hashCode()
        if (!force && hash == s.backupHash && s.backupError.isEmpty()) return false
        return try {
            val text = Backup.toJson(ctx, System.currentTimeMillis())
            val out = try {
                ctx.contentResolver.openOutputStream(uri, "wt")
            } catch (_: Exception) {
                ctx.contentResolver.openOutputStream(uri, "w") // some providers reject truncate mode
            } ?: error("provider refused the file")
            out.use { it.write(text.toByteArray()) }
            s.backupHash = hash
            s.backupLast = System.currentTimeMillis()
            s.backupError = ""
            Ticker.bump()
            true
        } catch (e: Exception) {
            s.backupError = e.message ?: e.javaClass.simpleName
            Ticker.bump()
            false
        }
    }
}

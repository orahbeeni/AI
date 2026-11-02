package com.omar.wifishare

import org.json.JSONArray
import org.json.JSONObject

data class Link(
    /** idle, unpaired, searching or connected. */
    val state: String,
    val computer: String?,
    val addr: String?,
    /** How long it has been connected, or searching. */
    val secs: Long,
    val rtt: Int?,
    val error: String?,
) {
    val connected get() = state == "connected"
}

data class Computer(val name: String, val current: Boolean, val lastAddr: String?)

/** A computer heard on this network. */
data class Heard(val name: String, val addr: String, val paired: Boolean)

/** A pairing code waiting for the user to compare it with the computer's. */
data class Prompt(val id: Long, val peer: String, val code: String, val addr: String)

data class Copy(
    val id: Long,
    val download: Boolean,
    val peer: String,
    val what: String,
    val place: String,
    val done: Long,
    val total: Long,
    /** running, waiting, done, failed or cancelled. */
    val state: String,
    val detail: String,
) {
    val finished get() = state == "done" || state == "failed" || state == "cancelled"
    val fraction get() = if (total <= 0) (if (state == "done") 1f else 0f) else (done.toFloat() / total).coerceIn(0f, 1f)
}

data class Notice(val id: Long, val text: String)

data class EngineState(
    val link: Link,
    val computers: List<Computer>,
    val discovered: List<Heard>,
    val prompts: List<Prompt>,
    val copies: List<Copy>,
    val notice: Notice?,
) {
    val current get() = computers.firstOrNull { it.current }

    companion object {
        val EMPTY = EngineState(Link("idle", null, null, 0, null, null), emptyList(), emptyList(), emptyList(), emptyList(), null)

        fun parse(text: String?): EngineState {
            if (text == null || !text.startsWith("{")) return EMPTY
            val o = JSONObject(text)
            val l = o.getJSONObject("link")
            return EngineState(
                link = Link(
                    state = l.getString("state"),
                    computer = l.optStr("computer"),
                    addr = l.optStr("addr"),
                    secs = l.optLong("secs"),
                    rtt = if (l.isNull("rtt") || !l.has("rtt")) null else l.getInt("rtt"),
                    error = l.optStr("error"),
                ),
                computers = o.getJSONArray("computers").objects().map {
                    Computer(it.getString("name"), it.getBoolean("current"), it.optStr("lastAddr"))
                },
                discovered = o.getJSONArray("discovered").objects().map {
                    Heard(it.getString("name"), it.getString("addr"), it.getBoolean("paired"))
                },
                prompts = o.getJSONArray("prompts").objects().map {
                    Prompt(it.getLong("id"), it.getString("peer"), it.getString("code"), it.getString("addr"))
                },
                copies = o.getJSONArray("jobs").objects().map {
                    Copy(
                        id = it.getLong("id"),
                        download = it.getString("kind") == "download",
                        peer = it.getString("peer"),
                        what = it.getString("what"),
                        place = it.getString("place"),
                        done = it.getLong("done"),
                        total = it.getLong("total"),
                        state = it.getString("state"),
                        detail = it.getString("detail"),
                    )
                },
                notice = o.optJSONObject("notice")?.let { Notice(it.getLong("id"), it.getString("text")) },
            )
        }
    }
}

data class ShareInfo(val name: String, val write: Boolean)

data class Entry(val name: String, val dir: Boolean, val size: Long, val modified: Long)

/** The answer to a request the computer answers: a value, or why not. */
sealed interface Answer<out T> {
    data class Ok<T>(val value: T) : Answer<T>
    data class Failed(val error: String) : Answer<Nothing>
}

fun <T> answer(text: String, read: (Any) -> T): Answer<T> {
    val o = JSONObject(text)
    return if (o.has("ok")) Answer.Ok(read(o.get("ok"))) else Answer.Failed(o.optString("error", "failed"))
}

fun parseShares(v: Any): List<ShareInfo> =
    (v as JSONArray).objects().map { ShareInfo(it.getString("name"), it.getBoolean("write")) }

fun parseEntries(v: Any): List<Entry> =
    (v as JSONArray).objects().map { Entry(it.getString("name"), it.getBoolean("dir"), it.getLong("size"), it.getLong("modified")) }

fun jsonList(items: List<String>): String = JSONArray(items).toString()

private fun JSONArray.objects(): List<JSONObject> = (0 until length()).map { getJSONObject(it) }

private fun JSONObject.optStr(key: String): String? = if (has(key) && !isNull(key)) getString(key) else null

/** 1.4 MB and the like. */
fun human(bytes: Long): String {
    val units = listOf("B", "KB", "MB", "GB", "TB")
    var v = bytes.toDouble()
    var u = 0
    while (v >= 1024 && u < units.size - 1) {
        v /= 1024
        u++
    }
    return if (u == 0) "$bytes B" else String.format("%.1f %s", v, units[u])
}

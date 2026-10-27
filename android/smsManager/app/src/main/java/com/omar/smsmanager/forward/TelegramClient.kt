package com.omar.smsmanager.forward

import org.json.JSONException
import org.json.JSONObject
import java.io.IOException
import java.net.HttpURLConnection
import java.net.URL
import java.net.URLEncoder

/** Telegram answered with an error. */
class TelegramException(val code: Int, message: String) : IOException(message) {
    /** Wrong or revoked token (401, 404), or the bot was blocked / removed from the chat (403). Retrying won't help. */
    val needsFix get() = code == 401 || code == 403 || code == 404

    /** This particular message was refused (for example it can't be delivered); drop it rather than blocking the rest. */
    val refused get() = code == 400
}

class TelegramChat(val id: Long, val name: String)

/** A way to POST a form and get back the status and body, so tests don't need a network. */
fun interface Http {
    fun post(url: String, form: Map<String, String>): Pair<Int, String>
}

object UrlHttp : Http {
    override fun post(url: String, form: Map<String, String>): Pair<Int, String> {
        val c = URL(url).openConnection() as HttpURLConnection
        try {
            c.connectTimeout = 15_000
            c.readTimeout = 20_000
            c.requestMethod = "POST"
            c.doOutput = true
            c.setRequestProperty("Content-Type", "application/x-www-form-urlencoded; charset=utf-8")
            val data = form.entries.joinToString("&") { enc(it.key) + "=" + enc(it.value) }.toByteArray(Charsets.UTF_8)
            c.outputStream.use { it.write(data) }
            val code = c.responseCode
            // Telegram explains its errors in the body of a 4xx reply.
            val body = (if (code in 200..299) c.inputStream else c.errorStream)?.bufferedReader()?.use { it.readText() }.orEmpty()
            return code to body
        } finally {
            c.disconnect()
        }
    }

    private fun enc(s: String) = URLEncoder.encode(s, "UTF-8")
}

/** The two Bot API calls we need. The token is only ever part of the request URL, never logged or put in an error. */
class TelegramClient(private val token: String, private val http: Http = UrlHttp, private val base: String = "https://api.telegram.org") {

    fun sendMessage(chatId: Long, text: String) {
        call("sendMessage", mapOf("chat_id" to chatId.toString(), "text" to text.take(MAX_TEXT), "disable_web_page_preview" to "true"))
    }

    /** People who have messaged the bot, one entry per private chat, newest information first. */
    fun findChats(): List<TelegramChat> {
        val result = call("getUpdates", mapOf("limit" to "100", "timeout" to "0"))
        val updates = result.optJSONArray("result") ?: return emptyList()
        val seen = LinkedHashMap<Long, TelegramChat>()
        for (i in updates.length() - 1 downTo 0) {
            val chat = updates.optJSONObject(i)?.optJSONObject("message")?.optJSONObject("chat") ?: continue
            if (chat.optString("type") != "private") continue
            val id = chat.optLong("id")
            if (id == 0L || id in seen) continue
            val name = listOf(chat.optString("first_name"), chat.optString("last_name")).filter { it.isNotEmpty() }.joinToString(" ")
            val user = chat.optString("username").let { if (it.isEmpty()) "" else " (@$it)" }
            seen[id] = TelegramChat(id, (name + user).ifEmpty { id.toString() })
        }
        return seen.values.toList()
    }

    private fun call(method: String, form: Map<String, String>): JSONObject {
        val (status, body) = http.post("$base/bot$token/$method", form)
        val json = try { JSONObject(body) } catch (_: JSONException) { throw IOException("unexpected reply from Telegram (HTTP $status)") }
        if (!json.optBoolean("ok")) throw TelegramException(json.optInt("error_code", status), json.optString("description"))
        return json
    }

    companion object {
        const val MAX_TEXT = 4096

        /** Looks like 123456789:AAH... */
        fun looksLikeToken(t: String) = Regex("""\d{5,}:[A-Za-z0-9_-]{20,}""").matches(t)
    }
}

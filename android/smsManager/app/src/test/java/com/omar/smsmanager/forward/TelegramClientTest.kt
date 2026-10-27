package com.omar.smsmanager.forward

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import java.io.IOException

class TelegramClientTest {
    private class Fake(val status: Int, val body: String) : Http {
        var url = ""
        var form: Map<String, String> = emptyMap()
        override fun post(url: String, form: Map<String, String>): Pair<Int, String> {
            this.url = url; this.form = form
            return status to body
        }
    }

    private val token = "123456789:AAH_example-token_0123456789abcdef"

    @Test fun sendsToTheRightChat() {
        val http = Fake(200, """{"ok":true,"result":{}}""")
        TelegramClient(token, http, "https://t.test").sendMessage(42, "hello")
        assertEquals("https://t.test/bot$token/sendMessage", http.url)
        assertEquals("42", http.form["chat_id"])
        assertEquals("hello", http.form["text"])
    }

    @Test fun longTextIsCutToTelegramsLimit() {
        val http = Fake(200, """{"ok":true}""")
        TelegramClient(token, http).sendMessage(1, "x".repeat(5000))
        assertEquals(TelegramClient.MAX_TEXT, http.form["text"]!!.length)
    }

    @Test fun badTokenNeedsFixing() {
        val http = Fake(401, """{"ok":false,"error_code":401,"description":"Unauthorized"}""")
        try {
            TelegramClient(token, http).sendMessage(1, "x")
            fail("expected failure")
        } catch (e: TelegramException) {
            assertTrue(e.needsFix); assertFalse(e.refused)
            assertFalse(Forwarder.describe(e).contains(token))
        }
    }

    @Test fun blockedBotNeedsFixing() {
        val http = Fake(403, """{"ok":false,"error_code":403,"description":"Forbidden: bot was blocked by the user"}""")
        try { TelegramClient(token, http).sendMessage(1, "x"); fail() } catch (e: TelegramException) { assertTrue(e.needsFix) }
    }

    @Test fun badRequestIsDroppedNotRetried() {
        val http = Fake(400, """{"ok":false,"error_code":400,"description":"Bad Request: chat not found"}""")
        try { TelegramClient(token, http).sendMessage(1, "x"); fail() } catch (e: TelegramException) {
            assertTrue(e.refused); assertFalse(e.needsFix)
        }
    }

    @Test fun rateLimitAndServerErrorsAreRetryable() {
        for (code in listOf(429, 500, 502)) {
            val http = Fake(code, """{"ok":false,"error_code":$code,"description":"later"}""")
            try { TelegramClient(token, http).sendMessage(1, "x"); fail() } catch (e: TelegramException) {
                assertFalse(e.needsFix); assertFalse(e.refused)
            }
        }
    }

    @Test fun garbageReplyIsAnIoError() {
        try { TelegramClient(token, Fake(502, "<html>Bad gateway</html>")).sendMessage(1, "x"); fail() } catch (e: IOException) {
            assertFalse(e is TelegramException)
        }
    }

    private fun updates(vararg chats: String) =
        """{"ok":true,"result":[${chats.joinToString(",") { """{"update_id":1,"message":{"chat":$it,"text":"hi"}}""" }}]}"""

    @Test fun findsTheOnePersonWhoMessagedTheBot() {
        val me = """{"id":555,"first_name":"Omar","username":"omar","type":"private"}"""
        val chats = TelegramClient(token, Fake(200, updates(me, me, me))).findChats()
        assertEquals(1, chats.size)
        assertEquals(555L, chats[0].id)
        assertEquals("Omar (@omar)", chats[0].name)
    }

    @Test fun strangersShowUpAsSeparateChats() {
        val me = """{"id":555,"first_name":"Omar","type":"private"}"""
        val other = """{"id":777,"first_name":"Eve","last_name":"Smith","username":"eve","type":"private"}"""
        val chats = TelegramClient(token, Fake(200, updates(me, other))).findChats()
        assertEquals(setOf(555L, 777L), chats.map { it.id }.toSet())
    }

    @Test fun groupsAreIgnored() {
        val group = """{"id":-100200,"title":"Some group","type":"supergroup"}"""
        assertTrue(TelegramClient(token, Fake(200, updates(group))).findChats().isEmpty())
    }

    @Test fun noMessagesMeansNoChats() {
        assertTrue(TelegramClient(token, Fake(200, """{"ok":true,"result":[]}""")).findChats().isEmpty())
    }

    @Test fun tokenShape() {
        assertTrue(TelegramClient.looksLikeToken(token))
        assertFalse(TelegramClient.looksLikeToken("not a token"))
        assertFalse(TelegramClient.looksLikeToken("$token\n"))
    }
}

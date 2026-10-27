package com.omar.smsmanager.forward

import com.omar.smsmanager.sms.OtpExtractor
import com.omar.smsmanager.spam.MatchMode
import com.omar.smsmanager.spam.PatternMatcher
import com.omar.smsmanager.spam.RuleTarget

/** Which messages get forwarded. Blocked messages never get this far. */
object ForwardPolicy {
    fun parseSenders(text: String): List<String> = text.lines().map { it.trim() }.filter { it.isNotEmpty() }

    /**
     * [onlySenders] uses the same patterns as sender rules (`*`, `?`, whole address, case-insensitive);
     * an empty list means everyone.
     */
    fun shouldForward(onlySenders: List<String>, skipCodes: Boolean, sender: String, body: String): Boolean {
        if (onlySenders.isNotEmpty()) {
            val address = PatternMatcher.normalizeSender(sender)
            val listed = onlySenders.any { PatternMatcher.compile(it, MatchMode.WILDCARD, RuleTarget.SENDER)?.matches(address) == true }
            if (!listed) return false
        }
        if (skipCodes && OtpExtractor.extract(body) != null) return false
        return true
    }
}

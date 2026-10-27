package com.omar.smsmanager.spam

import java.text.Normalizer
import java.util.regex.PatternSyntaxException

/**
 * Turns a user-written pattern into a [Regex]. Matching always ignores case.
 *
 * Wildcard: `*` = any run of characters (including none), `?` = exactly one character,
 * `\*` / `\?` / `\\` = the literal character. Everything else is literal.
 *
 * Loose: `*` = any run of characters, `?` = zero or one character. Both the pattern and the text are
 * reduced to lowercase letters and digits first (accents dropped), so "opt?out" matches "Opt-out",
 * "O.P.T out" and "optout". Separators are gone before matching, so a loose pattern can also match
 * across word boundaries ("free gift" matches "FREE-GIFT" and "f r e e g i f t").
 *
 * Regex: the pattern is used as written.
 */
object PatternMatcher {
    private val OPTIONS = setOf(RegexOption.IGNORE_CASE, RegexOption.DOT_MATCHES_ALL)

    /** Spaces, dashes, dots and parentheses carry no meaning in a phone number. */
    fun normalizeSender(s: String): String = s.filterNot { it.isWhitespace() || it in "-(). " }

    /** Lowercase letters and digits only, with accents stripped. */
    fun looseNormalize(s: String): String {
        val sb = StringBuilder()
        for (c in Normalizer.normalize(s, Normalizer.Form.NFD)) if (c.isLetterOrDigit()) sb.append(c.lowercaseChar())
        return sb.toString()
    }

    /** Returns null when [pattern] can't be compiled (empty, or an invalid regular expression). */
    fun compile(pattern: String, mode: MatchMode, target: RuleTarget): Regex? {
        if (pattern.isBlank()) return null
        return try {
            val source = when (mode) {
                MatchMode.REGEX -> pattern
                MatchMode.WILDCARD -> wildcardToRegex(if (target == RuleTarget.SENDER) normalizeSender(pattern) else pattern)
                MatchMode.LOOSE -> looseToRegex(pattern) ?: return null
            }
            Regex(source, OPTIONS)
        } catch (_: PatternSyntaxException) {
            null
        }
    }

    fun isValid(pattern: String, mode: MatchMode, target: RuleTarget) = compile(pattern, mode, target) != null

    /** Makes [s] match itself literally in Wildcard or Loose mode, by escaping `\`, `*` and `?`. */
    fun escapeWildcards(s: String): String {
        val sb = StringBuilder()
        for (c in s) {
            if (c == '\\' || c == '*' || c == '?') sb.append('\\')
            sb.append(c)
        }
        return sb.toString()
    }

    // ---- search box ----

    /** True when a search query uses `*` or `?`, so it should be matched as a wildcard rather than as plain text. */
    fun hasWildcards(q: String) = q.any { it == '*' || it == '?' }

    /**
     * A test for "does this text contain the query", used by the search box. Plain queries are a
     * case-insensitive substring; queries with `*` or `?` use the same wildcard syntax as rules.
     */
    fun searchMatcher(q: String): (String) -> Boolean {
        val re = if (hasWildcards(q)) compile(q, MatchMode.WILDCARD, RuleTarget.TEXT) else null
        return if (re != null) { s -> re.containsMatchIn(s) } else { s -> s.contains(q, ignoreCase = true) }
    }

    /**
     * The wildcard query as a SQL LIKE pattern (use with ESCAPE '\\'), without the surrounding `%`.
     * Used to narrow the database query; the exact match is then confirmed with [searchMatcher].
     */
    fun wildcardToLike(q: String): String {
        val sb = StringBuilder()
        var i = 0
        while (i < q.length) {
            val c = q[i]
            val literal = if (c == '\\' && i + 1 < q.length) q[++i] else null
            when {
                literal != null -> sb.append(if (literal in "%_\\") "\\$literal" else literal.toString())
                c == '*' -> sb.append('%')
                c == '?' -> sb.append('_')
                c in "%_\\" -> sb.append('\\').append(c)
                else -> sb.append(c)
            }
            i++
        }
        return sb.toString()
    }

    private fun wildcardToRegex(p: String): String {
        val sb = StringBuilder()
        var i = 0
        while (i < p.length) {
            val c = p[i]
            when {
                c == '\\' && i + 1 < p.length -> {
                    sb.append(Regex.escape(p[i + 1].toString()))
                    i++
                }
                c == '*' -> sb.append(".*")
                c == '?' -> sb.append('.')
                else -> sb.append(Regex.escape(c.toString()))
            }
            i++
        }
        return sb.toString()
    }

    private fun looseToRegex(p: String): String? {
        val sb = StringBuilder()
        val literal = StringBuilder()
        fun flush() {
            val n = looseNormalize(literal.toString())
            if (n.isNotEmpty()) sb.append(Regex.escape(n))
            literal.clear()
        }
        var i = 0
        while (i < p.length) {
            val c = p[i]
            when {
                c == '\\' && i + 1 < p.length -> {
                    literal.append(p[i + 1])
                    i++
                }
                c == '*' -> { flush(); sb.append(".*") }
                c == '?' -> { flush(); sb.append(".?") }
                else -> literal.append(c)
            }
            i++
        }
        flush()
        return sb.toString().ifEmpty { null }
    }
}

class CompiledRule(val rule: Rule, private val re: Regex) {
    private val loose = rule.mode == MatchMode.LOOSE

    fun matches(sender: String, body: String): Boolean = when (rule.target) {
        RuleTarget.SENDER -> re.matches(if (loose) PatternMatcher.looseNormalize(sender) else PatternMatcher.normalizeSender(sender))
        RuleTarget.TEXT -> re.containsMatchIn(if (loose) PatternMatcher.looseNormalize(body) else body)
    }
}

package com.omar.smsmanager.spam

enum class RuleAction { BLOCK, ALLOW }

/** SENDER rules match the whole sender address; TEXT rules match anywhere in the body. */
enum class RuleTarget { SENDER, TEXT }

/**
 * WILDCARD: `*` and `?` stand for text, everything else is literal.
 * LOOSE: the same wildcards, but spaces, punctuation, accents and case are ignored on both sides.
 * REGEX: a regular expression as written.
 */
enum class MatchMode { WILDCARD, LOOSE, REGEX }

data class Rule(
    val id: Long = 0,
    val action: RuleAction,
    val target: RuleTarget,
    val pattern: String,
    val mode: MatchMode = MatchMode.WILDCARD,
    val enabled: Boolean = true,
    val hits: Int = 0,
    val createdAt: Long = System.currentTimeMillis(),
) {
    val label: String
        get() = "${if (target == RuleTarget.SENDER) "Sender" else "Text"}: $pattern"
}

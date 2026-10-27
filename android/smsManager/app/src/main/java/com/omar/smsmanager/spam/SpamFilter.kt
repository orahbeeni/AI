package com.omar.smsmanager.spam

sealed interface Verdict {
    /** No rule applied. */
    data object Pass : Verdict

    /** Let through because of an allow rule or because the sender is a contact. */
    data class Allowed(val rule: Rule?) : Verdict

    data class Blocked(val rule: Rule) : Verdict
}

/** Order: allow rules, then contacts, then block rules. Disabled and invalid rules are ignored. */
class SpamFilter(rules: List<Rule>) {
    private val allow = compile(rules, RuleAction.ALLOW)
    private val block = compile(rules, RuleAction.BLOCK)

    fun evaluate(sender: String, body: String, isContact: Boolean = false): Verdict {
        allow.firstOrNull { it.matches(sender, body) }?.let { return Verdict.Allowed(it.rule) }
        if (isContact) return Verdict.Allowed(null)
        block.firstOrNull { it.matches(sender, body) }?.let { return Verdict.Blocked(it.rule) }
        return Verdict.Pass
    }

    private fun compile(rules: List<Rule>, action: RuleAction) = rules
        .filter { it.enabled && it.action == action }
        .mapNotNull { r -> PatternMatcher.compile(r.pattern, r.mode, r.target)?.let { CompiledRule(r, it) } }
}

package com.omar.smsmanager.spam

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class SpamFilterTest {
    private fun mode(regex: Boolean, loose: Boolean) = when {
        regex -> MatchMode.REGEX
        loose -> MatchMode.LOOSE
        else -> MatchMode.WILDCARD
    }

    private fun sender(p: String, regex: Boolean = false, action: RuleAction = RuleAction.BLOCK, id: Long = 1, loose: Boolean = false) =
        Rule(id = id, action = action, target = RuleTarget.SENDER, pattern = p, mode = mode(regex, loose))

    private fun text(p: String, regex: Boolean = false, action: RuleAction = RuleAction.BLOCK, id: Long = 1, loose: Boolean = false) =
        Rule(id = id, action = action, target = RuleTarget.TEXT, pattern = p, mode = mode(regex, loose))

    private fun blocked(rules: List<Rule>, from: String, body: String = "hello", contact: Boolean = false) =
        SpamFilter(rules).evaluate(from, body, contact) is Verdict.Blocked

    @Test fun senderPrefixWildcard() {
        val r = listOf(sender("+1555*"))
        assertTrue(blocked(r, "+15551234567"))
        assertFalse(blocked(r, "+4915551234567"))
    }

    @Test fun senderIsWholeStringNotSubstring() {
        val r = listOf(sender("555"))
        assertFalse(blocked(r, "+15551234567"))
        assertTrue(blocked(r, "555"))
    }

    @Test fun senderFormattingIsIgnoredOnBothSides() {
        assertTrue(blocked(listOf(sender("+1 (555) 123-*")), "+15551234567"))
        assertTrue(blocked(listOf(sender("+15551234567")), "+1 555 123 4567"))
    }

    @Test fun senderSuffixAndAlphanumeric() {
        assertTrue(blocked(listOf(sender("*4567")), "+15551234567"))
        assertTrue(blocked(listOf(sender("promo*")), "PROMO-DEALS"))
        assertTrue(blocked(listOf(sender("promo*")), "PromoDeals"))
    }

    @Test fun questionMarkMatchesExactlyOneChar() {
        val r = listOf(sender("+1555123456?"))
        assertTrue(blocked(r, "+15551234567"))
        assertFalse(blocked(r, "+155512345"))
        assertFalse(blocked(r, "+155512345678"))
    }

    @Test fun textMatchesAnywhereAndIgnoresCase() {
        val r = listOf(text("free gift"))
        assertTrue(blocked(r, "x", "Claim your FREE GIFT now!"))
        assertFalse(blocked(r, "x", "Claim your gift for free"))
    }

    @Test fun textWildcardSpansLinesAndWords() {
        val r = listOf(text("click*http"))
        assertTrue(blocked(r, "x", "Please click here:\nhttp://evil.example"))
        assertFalse(blocked(r, "x", "http first, click later"))
    }

    @Test fun escapedWildcardsAreLiteral() {
        val r = listOf(text("50\\*off"))
        assertTrue(blocked(r, "x", "get 50*off today"))
        assertFalse(blocked(r, "x", "get 50% off today"))
        assertTrue(blocked(listOf(text("really\\?")), "x", "really?"))
        assertFalse(blocked(listOf(text("really\\?")), "x", "reallyx"))
    }

    @Test fun regexSpecialCharsAreLiteralInWildcardMode() {
        assertTrue(blocked(listOf(text("a.b(c)[d]+")), "x", "xx a.b(c)[d]+ xx"))
        assertFalse(blocked(listOf(text("a.b")), "x", "axb"))
    }

    @Test fun regexMode() {
        val r = listOf(text("\\b\\d{6}\\b", regex = true))
        assertTrue(blocked(r, "x", "code 123456 here"))
        assertFalse(blocked(r, "x", "code 12345 here"))
        assertTrue(blocked(listOf(sender("\\+44\\d{10}", regex = true)), "+447700900123"))
        assertFalse(blocked(listOf(sender("\\+44\\d{10}", regex = true)), "+4477009001234"))
    }

    @Test fun invalidAndBlankPatternsNeverMatch() {
        assertNull(PatternMatcher.compile("(", MatchMode.REGEX, RuleTarget.TEXT))
        assertNull(PatternMatcher.compile("  ", MatchMode.WILDCARD, RuleTarget.TEXT))
        assertFalse(blocked(listOf(text("(", regex = true), text("")), "x", "anything ( at all"))
    }

    @Test fun allowRuleBeatsBlockRule() {
        val r = listOf(
            sender("*", id = 1),
            sender("+1555000*", action = RuleAction.ALLOW, id = 2),
        )
        val v = SpamFilter(r).evaluate("+15550001111", "hi")
        assertTrue(v is Verdict.Allowed)
        assertEquals(2L, (v as Verdict.Allowed).rule?.id)
    }

    @Test fun textAllowRuleCanRescueOtpFromBlockedSender() {
        val r = listOf(
            sender("+1555*", id = 1),
            text("your code is*", action = RuleAction.ALLOW, id = 2),
        )
        assertFalse(blocked(r, "+15551111111", "Your code is 482913"))
        assertTrue(blocked(r, "+15551111111", "Buy now"))
    }

    @Test fun contactsBypassBlockRules() {
        val r = listOf(text("free"))
        assertTrue(blocked(r, "x", "free stuff", contact = false))
        assertFalse(blocked(r, "x", "free stuff", contact = true))
    }

    @Test fun disabledRulesAreIgnored() {
        assertFalse(blocked(listOf(text("free").copy(enabled = false)), "x", "free"))
    }

    @Test fun reportsWhichRuleBlocked() {
        val v = SpamFilter(listOf(text("nope", id = 1), text("free", id = 7))).evaluate("x", "so free")
        assertEquals(7L, (v as Verdict.Blocked).rule.id)
    }

    @Test fun passWhenNothingMatches() {
        assertEquals(Verdict.Pass, SpamFilter(listOf(text("free"))).evaluate("x", "hello"))
    }

    @Test fun plainTextRuleEqualsStarWrappedRule() {
        val bare = listOf(text("Optout"))
        val wrapped = listOf(text("*Optout*"))
        for (msg in listOf("optout", "OPTOUT now", "To OptOut reply STOP", "Reply optouts", "Opt-out", "opt out", "nothing")) {
            assertEquals(msg, blocked(wrapped, "x", msg), blocked(bare, "x", msg))
        }
        assertTrue(blocked(bare, "x", "To OPTOUT reply STOP"))
        assertFalse(blocked(bare, "x", "Opt-out"))
    }

    @Test fun caseInsensitivityCoversNonAsciiLetters() {
        assertTrue(blocked(listOf(text("gratis für sie")), "x", "GRATIS FÜR SIE!"))
        assertTrue(blocked(listOf(text("бесплатно")), "x", "БЕСПЛАТНО сегодня"))
    }

    // ---- loose mode ----

    @Test fun looseIgnoresSeparatorsAndCase() {
        val r = listOf(text("opt?out", loose = true))
        for (msg in listOf("optout", "Opt-out", "OPT OUT", "O.P.T out", "opt_out", "optXout", "Reply STOP to opt-out")) {
            assertTrue(msg, blocked(r, "x", msg))
        }
        assertFalse(blocked(r, "x", "opt xx out"))
        assertFalse(blocked(r, "x", "option"))
    }

    @Test fun looseQuestionMarkIsZeroOrOneCharacter() {
        assertTrue(blocked(listOf(text("colo?r", loose = true)), "x", "colour"))
        assertTrue(blocked(listOf(text("colo?r", loose = true)), "x", "color"))
        assertFalse(blocked(listOf(text("colo?r", loose = true)), "x", "colouur"))
    }

    @Test fun looseStarSpansAnything() {
        val r = listOf(text("free*gift", loose = true))
        assertTrue(blocked(r, "x", "FREE!!! amazing, unbelievable GIFT"))
        assertFalse(blocked(r, "x", "gift then free"))
    }

    @Test fun looseDropsAccents() {
        assertTrue(blocked(listOf(text("gratis fur sie", loose = true)), "x", "GRATIS FÜR SIE"))
        assertTrue(blocked(listOf(text("gratis für sie", loose = true)), "x", "gratis fur sie"))
    }

    @Test fun looseSenderIgnoresPlusAndFormatting() {
        val r = listOf(sender("+1 (900) *", loose = true))
        assertTrue(blocked(r, "+19005550101"))
        assertTrue(blocked(r, "19005550101"))
        assertFalse(blocked(r, "+4419005550101"))
        assertTrue(blocked(listOf(sender("promo*", loose = true)), "P-R-O-M-O Deals"))
    }

    @Test fun looseWithNothingToMatchIsInvalid() {
        assertNull(PatternMatcher.compile("--- !!!", MatchMode.LOOSE, RuleTarget.TEXT))
        assertNull(PatternMatcher.compile("   ", MatchMode.LOOSE, RuleTarget.TEXT))
        assertTrue(PatternMatcher.isValid("*", MatchMode.LOOSE, RuleTarget.TEXT))
    }

    // ---- search box ----

    @Test fun searchPlainQueryIsCaseInsensitiveSubstring() {
        val m = PatternMatcher.searchMatcher("lunch")
        assertTrue(m("Having LUNCH at noon"))
        assertFalse(m("brunch"))
        assertTrue(PatternMatcher.searchMatcher("a.b")("xa.by"))
        assertFalse(PatternMatcher.searchMatcher("a.b")("axb"))
    }

    @Test fun searchWildcards() {
        assertTrue(PatternMatcher.searchMatcher("lun*noon")("Lunch at noon"))
        assertFalse(PatternMatcher.searchMatcher("lun*noon")("noon then lunch"))
        assertTrue(PatternMatcher.searchMatcher("c?t")("the cat sat"))
        assertFalse(PatternMatcher.searchMatcher("c?t")("ct"))
        assertTrue(PatternMatcher.searchMatcher("50\\*")("save 50* now"))
        assertFalse(PatternMatcher.searchMatcher("50\\*")("save 500 now"))
        assertTrue(PatternMatcher.searchMatcher("+27*")("call +27 82 555 0100"))
    }

    @Test fun wildcardToLike() {
        assertEquals("lun%noon", PatternMatcher.wildcardToLike("lun*noon"))
        assertEquals("c_t", PatternMatcher.wildcardToLike("c?t"))
        assertEquals("100\\% sure%", PatternMatcher.wildcardToLike("100% sure*"))
        assertEquals("a\\_b_", PatternMatcher.wildcardToLike("a_b?"))
        assertEquals("50*", PatternMatcher.wildcardToLike("50\\*"))
        assertEquals("what?", PatternMatcher.wildcardToLike("what\\?"))
        assertEquals("back\\\\slash", PatternMatcher.wildcardToLike("back\\\\slash"))
        assertEquals("trailing\\\\", PatternMatcher.wildcardToLike("trailing\\"))
    }
}

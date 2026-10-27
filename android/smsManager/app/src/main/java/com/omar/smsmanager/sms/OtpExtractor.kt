package com.omar.smsmanager.sms

/**
 * Finds a one-time code in a message, or returns null when the text doesn't look like one.
 *
 * A message only counts if it contains a word like "code", "OTP" or "PIN", and the code chosen is the
 * candidate nearest to that word, preferring one that follows it ("code is 482913") over one that precedes
 * it ("482913 is your code"). That keeps amounts, order numbers and phone numbers from being picked up.
 */
object OtpExtractor {
    private val KEYWORD = Regex(
        """(?<![a-z])(code|otp|pin|passcode|password|verification|verify|one[- ]?time|token|2fa|authentication|kode|codice|c[oó]digo|wachtwoord)(?![a-z])""",
        RegexOption.IGNORE_CASE,
    )

    // 4-8 digits, or digits split in two by a space or dash ("123 456"), not glued to letters or digits.
    private val DIGITS = Regex("""(?<![A-Za-z0-9])(\d{3}[ -]\d{3}|\d{4}[ -]\d{4}|\d{4,8})(?![A-Za-z0-9])""")

    // Mixed letters and digits ("G7K2P9"), 5-8 characters, with at least two letters (checked below).
    private val MIXED = Regex("""(?<![A-Za-z0-9])(?=[A-Za-z0-9]*\d)(?=[A-Za-z0-9]*[A-Za-z])[A-Za-z0-9]{5,8}(?![A-Za-z0-9])""")

    private val CURRENCY_BEFORE = Regex("""(?i)(\bR|\bZAR|\bUSD|\bEUR|\bGBP|[$€£+#])\s*$""")
    private const val MAX_DISTANCE = 80
    private const val BEFORE_PENALTY = 20

    private class Candidate(val start: Int, val end: Int, val text: String)

    fun extract(body: String): String? {
        val keywords = KEYWORD.findAll(body).toList()
        if (keywords.isEmpty()) return null

        val candidates = ArrayList<Candidate>()
        for (m in DIGITS.findAll(body)) {
            if (looksLikeAmountOrNumber(body, m.range.first, m.range.last + 1)) continue
            candidates.add(Candidate(m.range.first, m.range.last + 1, m.value.filter { it.isDigit() }))
        }
        for (m in MIXED.findAll(body)) {
            // One letter plus digits is an amount like "R1500", not a code.
            if (m.value.count { it.isLetter() } < 2) continue
            candidates.add(Candidate(m.range.first, m.range.last + 1, m.value))
        }

        var best: Candidate? = null
        var bestDistance = Int.MAX_VALUE
        for (c in candidates) {
            for (k in keywords) {
                val d = if (c.start >= k.range.last + 1) c.start - (k.range.last + 1) else k.range.first - c.end + BEFORE_PENALTY
                if (d < bestDistance) { bestDistance = d; best = c }
            }
        }
        return if (bestDistance <= MAX_DISTANCE) best?.text else null
    }

    /** "R1,500.00", "$25", "99%", "12.50", "+27..." are not codes. */
    private fun looksLikeAmountOrNumber(s: String, start: Int, end: Int): Boolean {
        val before = s.substring(0, start)
        if (CURRENCY_BEFORE.containsMatchIn(before)) return true
        if (start >= 2 && (s[start - 1] == ',' || s[start - 1] == '.') && s[start - 2].isDigit()) return true
        if (end < s.length) {
            if (s[end] == '%') return true
            if ((s[end] == ',' || s[end] == '.') && end + 1 < s.length && s[end + 1].isDigit()) return true
        }
        return false
    }
}

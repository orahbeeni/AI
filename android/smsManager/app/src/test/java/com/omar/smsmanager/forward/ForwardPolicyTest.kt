package com.omar.smsmanager.forward

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ForwardPolicyTest {
    private fun ok(senders: String, skipCodes: Boolean, sender: String, body: String = "hello") =
        ForwardPolicy.shouldForward(ForwardPolicy.parseSenders(senders), skipCodes, sender, body)

    @Test fun emptyListForwardsEveryone() {
        assertTrue(ok("", false, "+27821234567"))
        assertTrue(ok("  \n \n", false, "ANYBODY"))
    }

    @Test fun listedSendersOnly() {
        assertTrue(ok("MYBANK", false, "mybank"))
        assertTrue(ok("+2782*", false, "+27 82 123 4567"))
        assertFalse(ok("MYBANK", false, "OTHERBANK"))
        assertTrue(ok("A\nMYBANK\nB", false, "MYBANK"))
        assertFalse(ok("MYBANK", false, "MYBANK-PROMO")) // whole address, like sender rules
    }

    @Test fun codesCanBeKeptOnThePhone() {
        val otp = "Your verification code is 482913"
        assertTrue(ok("", false, "BANK", otp))
        assertFalse(ok("", true, "BANK", otp))
        assertTrue(ok("", true, "BANK", "See you at 5"))
    }

    @Test fun parseTrimsAndDropsBlankLines() {
        assertEquals(listOf("A", "B"), ForwardPolicy.parseSenders(" A \n\nB\n"))
    }
}

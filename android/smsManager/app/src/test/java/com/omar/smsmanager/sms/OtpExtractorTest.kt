package com.omar.smsmanager.sms

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class OtpExtractorTest {
    private fun code(s: String) = OtpExtractor.extract(s)

    @Test fun commonFormats() {
        assertEquals("482913", code("Your verification code is 482913. Do not share it."))
        assertEquals("123456", code("G-123456 is your Google verification code."))
        assertEquals("4821", code("Your OTP is 4821. Valid for 5 minutes."))
        assertEquals("739201", code("739201 is your Uber code. Never share it."))
        assertEquals("0482", code("Use PIN 0482 to log in"))
    }

    @Test fun codeSplitInTwo() {
        assertEquals("123456", code("Your code is 123 456"))
        assertEquals("123456", code("Your code: 123-456"))
        assertEquals("12345678", code("Security code 1234-5678"))
    }

    @Test fun mixedLettersAndDigits() {
        assertEquals("G7K2P9", code("Your code is G7K2P9, enter it to continue"))
        assertEquals("8X2K9P", code("verification code: 8X2K9P"))
    }

    @Test fun bankStyleMessagesIgnoreAmounts() {
        assertEquals("12345", code("FNB: OTP 12345 for payment of R2,500.00 at Takealot"))
        assertEquals("1234", code("Use 1234 as your OTP for Capitec transaction of R1500.00"))
        assertEquals("889977", code("Your one time pin is 889977 to approve a payment of R 450"))
        assertEquals("4455", code("Authorise $250 purchase with code 4455"))
    }

    @Test fun picksTheNumberNextToTheKeyword() {
        assertEquals("7788", code("Your Takealot order 5521 will arrive soon. OTP is 7788"))
        assertEquals("5566", code("Order 12345678 shipped. Code: 5566"))
    }

    @Test fun nothingWithoutAKeyword() {
        assertNull(code("Hi mom, call me on 0821234567"))
        assertNull(code("Your parcel 482913 is ready for collection at 14:30"))
        assertNull(code("Dinner at 7? I booked table 4521"))
    }

    @Test fun nothingWithoutANumber() {
        assertNull(code("Please enter your code on the website"))
        assertNull(code("Your password was changed"))
    }

    @Test fun notFooledBySimilarWords() {
        assertNull(code("Free shipping on orders over 1500 items this week")) // "shipping" is not "pin"
        assertNull(code("The decoder 4821 is installed"))                      // "decoder" is not "code"
    }

    @Test fun ignoresPercentagesAndDecimals() {
        assertNull(code("Your code balance is 12.50 and 99% done"))
    }
}

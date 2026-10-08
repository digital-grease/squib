package net.digitalgrease.squib.data

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class IssueReportTest {
    @Test
    fun deviceNamesAreDeduplicated() {
        assertEquals("Google Pixel 8", IssueReport.formatDevice("google", "Pixel 8"))
        assertEquals("samsung Galaxy S24", IssueReport.formatDevice("samsung", "samsung Galaxy S24"))
        assertEquals("Samsung Galaxy S24", IssueReport.formatDevice("samsung", "Galaxy S24"))
        assertEquals("Pixel", IssueReport.formatDevice("", "Pixel"))
    }

    @Test
    fun shortReportsAreEmbeddedWithFieldIds() {
        val r = IssueReport.build("Pixel 8", "15 (API 35)", "0.1.0", "line one\nline two")
        assertTrue(r is IssueReport.Result.Embedded)
        assertTrue(r.url.startsWith("https://github.com/digital-grease/squib/issues/new?template=bug_report.yml"))
        for (id in listOf("device=", "android_version=", "app_version=", "diagnostics=")) assertTrue(id, r.url.contains("&$id"))
        assertTrue(r.url.contains("line%20one%0Aline%20two"))
    }

    @Test
    fun longReportsAreTruncatedWithinTheLimitAndKeptWhole() {
        val long = "x".repeat(20_000)
        val r = IssueReport.build("Pixel 8", "15", "0.1.0", long)
        assertTrue(r is IssueReport.Result.Truncated)
        assertTrue(r.url.length <= IssueReport.MAX_URL_LENGTH)
        assertEquals(long, (r as IssueReport.Result.Truncated).fullReport)
        assertTrue(r.url.contains("truncated"))
    }
}

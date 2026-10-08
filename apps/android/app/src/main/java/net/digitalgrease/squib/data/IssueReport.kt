package net.digitalgrease.squib.data

import java.net.URLEncoder

/**
 * Builds a pre-filled GitHub Issue Form URL for `.github/ISSUE_TEMPLATE/bug_report.yml`
 * (same approach as fauxx). Field ids in the URL match the form's field ids. The user
 * reviews and submits the issue in the browser; nothing is sent automatically.
 * Pure Kotlin so it can be unit tested without Android.
 */
object IssueReport {
    private const val BASE = "https://github.com/digital-grease/squib/issues/new?template=bug_report.yml"

    /** GitHub starts failing on very long query strings; stay well below. */
    const val MAX_URL_LENGTH = 7000
    private const val MARKER = "\n\n[...truncated: the full report was copied to the clipboard; paste it below]"

    sealed class Result {
        abstract val url: String
        data class Embedded(override val url: String) : Result()
        data class Truncated(override val url: String, val fullReport: String) : Result()
    }

    fun formatDevice(manufacturer: String, model: String): String {
        val mfg = manufacturer.trim()
        val mdl = model.trim()
        return when {
            mfg.isEmpty() -> mdl
            mdl.isEmpty() -> mfg.replaceFirstChar { it.titlecase() }
            mdl.startsWith(mfg, ignoreCase = true) -> mdl
            else -> "${mfg.replaceFirstChar { it.titlecase() }} $mdl"
        }
    }

    private fun enc(s: String) = URLEncoder.encode(s, "UTF-8").replace("+", "%20")

    fun build(device: String, androidVersion: String, appVersion: String, report: String, maxUrlLength: Int = MAX_URL_LENGTH): Result {
        val head = "$BASE&device=${enc(device)}&android_version=${enc(androidVersion)}&app_version=${enc(appVersion)}&diagnostics="
        val full = enc(report)
        if (head.length + full.length <= maxUrlLength) return Result.Embedded(head + full)
        // Shrink the embedded part until it fits, keeping the start of the report.
        var keep = report.length
        while (keep > 0) {
            keep = (keep * 0.8).toInt()
            val candidate = enc(report.take(keep) + MARKER)
            if (head.length + candidate.length <= maxUrlLength) return Result.Truncated(head + candidate, report)
        }
        return Result.Truncated(head + enc(MARKER), report)
    }
}

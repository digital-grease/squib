package dev.digitalgrease.squib.conditions

/**
 * When to refresh conditions without being asked (plan phase 3.2, docs/squib/05 "Freshness,
 * refresh, and offline"): only while the Timer or Conditions screen is in front, only with
 * weather lookup on and a place chosen, never during a run, and at most every 15 minutes
 * counted from the last attempt (so failures do not cause a burst of retries). There is no
 * background refresh or location tracking.
 */
object RefreshPolicy {
    const val INTERVAL_MS = 15 * 60_000L

    fun due(
        nowUtcMs: Long,
        lastRefreshUtcMs: Long?,
        lastAutoAttemptUtcMs: Long?,
        weatherEnabled: Boolean,
        hasPlace: Boolean,
        refreshing: Boolean,
        runActive: Boolean,
    ): Boolean {
        if (!weatherEnabled || !hasPlace || refreshing || runActive) return false
        val last = listOfNotNull(lastRefreshUtcMs, lastAutoAttemptUtcMs).maxOrNull() ?: return true
        // A clock set backwards does not stop refreshes.
        return nowUtcMs - last >= INTERVAL_MS || nowUtcMs < last
    }
}

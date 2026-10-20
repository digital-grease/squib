package dev.digitalgrease.squib.conditions

import java.io.ByteArrayOutputStream
import java.net.ConnectException
import java.net.HttpURLConnection
import java.net.SocketTimeoutException
import java.net.URL
import java.net.UnknownHostException
import javax.net.ssl.SSLException
import kotlin.random.Random
import dev.digitalgrease.squib.core.HttpRequestFfi
import dev.digitalgrease.squib.core.HttpResponseFfi

/**
 * Plain HTTPS GET for requests the core has already chosen and origin-checked.
 * Platform TLS verification, no redirects (a redirect could leave the allowed origin),
 * bounded body, typed transport errors. Never called on the audio path.
 */
object WeatherTransport {
    private const val CONNECT_TIMEOUT_MS = 8_000
    private const val READ_TIMEOUT_MS = 10_000

    /**
     * Retry-After as seconds from now: either a number of seconds or an HTTP date
     * (RFC 9110). A date in the past means "now"; anything unreadable is ignored.
     */
    fun retryAfterSeconds(header: String?, nowUtcMs: Long): UInt? {
        val v = header?.trim()?.takeIf { it.isNotEmpty() } ?: return null
        v.toUIntOrNull()?.let { return it }
        return runCatching {
            val at = java.time.ZonedDateTime.parse(v, java.time.format.DateTimeFormatter.RFC_1123_DATE_TIME).toInstant().toEpochMilli()
            ((at - nowUtcMs + 999) / 1000).coerceIn(0, Int.MAX_VALUE.toLong()).toUInt()
        }.getOrNull()
    }

    fun execute(req: HttpRequestFfi): HttpResponseFfi {
        val jitter = Random.nextDouble()
        fun fail(e: String) = HttpResponseFfi(req.id, 0u, ByteArray(0), null, null, null, e, jitter)
        if (!req.url.startsWith("https://")) return fail("not https")
        val conn = try {
            URL(req.url).openConnection() as HttpURLConnection
        } catch (e: Exception) {
            return fail("bad url")
        }
        return try {
            conn.instanceFollowRedirects = false
            conn.connectTimeout = CONNECT_TIMEOUT_MS
            // The core sets a longer timeout for slow services (terrain); 0 means default.
            conn.readTimeout = if (req.timeoutMs > 0u) req.timeoutMs.toInt() else READ_TIMEOUT_MS
            conn.useCaches = false
            conn.setRequestProperty("User-Agent", req.userAgent)
            conn.setRequestProperty("Accept", req.accept)
            req.ifNoneMatch?.let { conn.setRequestProperty("If-None-Match", it) }
            val status = conn.responseCode
            val limit = req.maxBodyBytes.toInt()
            val stream = if (status >= 400) conn.errorStream else conn.inputStream
            val out = ByteArrayOutputStream()
            var tooLarge = false
            stream?.use { s ->
                val buf = ByteArray(16 * 1024)
                while (true) {
                    val n = s.read(buf)
                    if (n < 0) break
                    if (out.size() + n > limit) {
                        tooLarge = true
                        break
                    }
                    out.write(buf, 0, n)
                }
            }
            if (tooLarge) return fail("too_large")
            val maxAge = conn.getHeaderField("Cache-Control")
                ?.let { Regex("max-age=(\\d+)").find(it)?.groupValues?.get(1)?.toUIntOrNull() }
            val retryAfter = retryAfterSeconds(conn.getHeaderField("Retry-After"), System.currentTimeMillis())
            HttpResponseFfi(req.id, status.toUShort(), out.toByteArray(), conn.getHeaderField("ETag"), maxAge, retryAfter, null, jitter)
        } catch (e: UnknownHostException) {
            fail("offline")
        } catch (e: ConnectException) {
            fail("offline")
        } catch (e: SocketTimeoutException) {
            fail("timeout")
        } catch (e: SSLException) {
            fail("tls")
        } catch (e: Exception) {
            fail(e.javaClass.simpleName)
        } finally {
            conn.disconnect()
        }
    }
}

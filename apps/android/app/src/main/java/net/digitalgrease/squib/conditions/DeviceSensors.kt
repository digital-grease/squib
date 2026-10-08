package net.digitalgrease.squib.conditions

import android.Manifest
import android.annotation.SuppressLint
import android.content.Context
import android.content.pm.PackageManager
import android.hardware.Sensor
import android.hardware.SensorEvent
import android.hardware.SensorEventListener
import android.hardware.SensorManager
import android.location.Location
import android.location.LocationManager
import android.os.Build
import android.os.CancellationSignal
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit

/**
 * One-shot foreground reads of the phone barometer and location. No background
 * tracking. Location heights are reported with their reference: `altitude` is WGS84
 * ellipsoid height; MSL comes only from `getMslAltitudeMeters()` where available.
 */
class DeviceSensors(private val context: Context) {
    data class Fix(
        val lat: Double,
        val lon: Double,
        val utcMs: Long,
        val ellipsoidM: Double?,
        val ellipsoidAccuracyM: Double?,
        val mslM: Double?,
        val mslAccuracyM: Double?,
        val precise: Boolean,
    )

    data class Pressure(val hpa: Double, val utcMs: Long)

    fun hasBarometer(): Boolean =
        (context.getSystemService(Context.SENSOR_SERVICE) as SensorManager).getDefaultSensor(Sensor.TYPE_PRESSURE) != null

    fun locationGranted(): Boolean =
        context.checkSelfPermission(Manifest.permission.ACCESS_COARSE_LOCATION) == PackageManager.PERMISSION_GRANTED ||
            context.checkSelfPermission(Manifest.permission.ACCESS_FINE_LOCATION) == PackageManager.PERMISSION_GRANTED

    private fun fineGranted() =
        context.checkSelfPermission(Manifest.permission.ACCESS_FINE_LOCATION) == PackageManager.PERMISSION_GRANTED

    /** Average up to ~1 s of barometer samples. Blocks the caller; call off the main thread. */
    fun readBarometer(): Pressure? {
        val sm = context.getSystemService(Context.SENSOR_SERVICE) as SensorManager
        val sensor = sm.getDefaultSensor(Sensor.TYPE_PRESSURE) ?: return null
        val samples = mutableListOf<Float>()
        val done = CountDownLatch(1)
        val listener = object : SensorEventListener {
            override fun onSensorChanged(e: SensorEvent) {
                synchronized(samples) {
                    samples += e.values[0]
                    if (samples.size >= 10) done.countDown()
                }
            }

            override fun onAccuracyChanged(s: Sensor?, a: Int) {}
        }
        sm.registerListener(listener, sensor, SensorManager.SENSOR_DELAY_NORMAL)
        done.await(1500, TimeUnit.MILLISECONDS)
        sm.unregisterListener(listener)
        val mean = synchronized(samples) { if (samples.isEmpty()) null else samples.average() } ?: return null
        return Pressure(mean, System.currentTimeMillis())
    }

    /** Single current fix within ~15 s; falls back to the last known fix if recent. */
    @SuppressLint("MissingPermission")
    fun currentFix(): Fix? {
        if (!locationGranted()) return null
        val lm = context.getSystemService(Context.LOCATION_SERVICE) as LocationManager
        val provider = when {
            fineGranted() && lm.isProviderEnabled(LocationManager.GPS_PROVIDER) -> LocationManager.GPS_PROVIDER
            lm.isProviderEnabled(LocationManager.NETWORK_PROVIDER) -> LocationManager.NETWORK_PROVIDER
            lm.isProviderEnabled(LocationManager.GPS_PROVIDER) -> LocationManager.GPS_PROVIDER
            else -> return null
        }
        var loc: Location? = null
        if (Build.VERSION.SDK_INT >= 30) {
            val latch = CountDownLatch(1)
            val cancel = CancellationSignal()
            val exec = Executors.newSingleThreadExecutor()
            lm.getCurrentLocation(provider, cancel, exec) { l ->
                loc = l
                latch.countDown()
            }
            if (!latch.await(15, TimeUnit.SECONDS)) cancel.cancel()
            exec.shutdown()
        }
        if (loc == null) {
            loc = lm.getLastKnownLocation(provider)?.takeIf { System.currentTimeMillis() - it.time < 10 * 60_000 }
        }
        val l = loc ?: return null
        val msl = if (Build.VERSION.SDK_INT >= 34 && l.hasMslAltitude()) l.mslAltitudeMeters else null
        val mslAcc = if (Build.VERSION.SDK_INT >= 34 && l.hasMslAltitudeAccuracy()) l.mslAltitudeAccuracyMeters.toDouble() else null
        return Fix(
            lat = l.latitude,
            lon = l.longitude,
            utcMs = l.time,
            ellipsoidM = if (l.hasAltitude()) l.altitude else null,
            ellipsoidAccuracyM = if (l.hasVerticalAccuracy()) l.verticalAccuracyMeters.toDouble() else null,
            mslM = msl,
            mslAccuracyM = mslAcc,
            precise = fineGranted(),
        )
    }
}

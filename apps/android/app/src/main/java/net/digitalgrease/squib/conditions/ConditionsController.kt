package net.digitalgrease.squib.conditions

import android.app.Application
import android.content.Context
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import java.util.concurrent.Executors
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import net.digitalgrease.squib.SquibApp
import net.digitalgrease.squib.core.ConditionsView
import net.digitalgrease.squib.core.DeviceReadings
import net.digitalgrease.squib.core.HttpRequestFfi
import net.digitalgrease.squib.core.PlaceInput
import net.digitalgrease.squib.core.SavedPlaceView
import net.digitalgrease.squib.core.SquibException

/**
 * Conditions screen state. All engine calls run on one worker thread; HTTP requests the
 * core asks for run on the IO pool (the core never has more than two outstanding).
 * Nothing here touches the audio path.
 */
class ConditionsController(app: Application) : AndroidViewModel(app) {
    private val engine = (app as SquibApp).engine
    private val sensors = DeviceSensors(app)
    private val worker = Executors.newSingleThreadExecutor { Thread(it, "squib-conditions") }.asCoroutineDispatcher()
    private val prefs = app.getSharedPreferences("conditions_ui", Context.MODE_PRIVATE)

    private val _view = MutableStateFlow<ConditionsView?>(null)
    val view: StateFlow<ConditionsView?> = _view.asStateFlow()
    private val _places = MutableStateFlow<List<SavedPlaceView>>(emptyList())
    val places: StateFlow<List<SavedPlaceView>> = _places.asStateFlow()
    private val _busy = MutableStateFlow<String?>(null)
    val busy: StateFlow<String?> = _busy.asStateFlow()
    private val _message = MutableStateFlow<String?>(null)
    val message: StateFlow<String?> = _message.asStateFlow()
    private val _us = MutableStateFlow(prefs.getBoolean("us_units", java.util.Locale.getDefault().country == "US"))
    /** US customary display units (°F, mph, inHg, ft); SI values are unchanged. */
    val usUnits: StateFlow<Boolean> = _us.asStateFlow()

    val hasBarometer = sensors.hasBarometer()

    init {
        reload()
    }

    fun setUsUnits(us: Boolean) {
        _us.value = us
        prefs.edit().putBoolean("us_units", us).apply()
    }

    fun clearMessage() {
        _message.value = null
    }

    fun reload() {
        viewModelScope.launch(worker) {
            _view.value = engine.conditionsView(System.currentTimeMillis())
            _places.value = engine.listSavedPlaces()
        }
    }

    private fun act(label: String?, f: suspend () -> Unit) {
        viewModelScope.launch(worker) {
            _busy.value = label
            try {
                f()
            } catch (e: SquibException) {
                _message.value = e.message
            } finally {
                _busy.value = null
                _view.value = engine.conditionsView(System.currentTimeMillis())
                _places.value = engine.listSavedPlaces()
            }
        }
    }

    fun setWeatherEnabled(on: Boolean) = act(null) { engine.setWeatherEnabled(on) }

    fun setWeatherProvider(p: String) = act(null) { engine.setWeatherProvider(p) }

    fun setPreciseRetention(on: Boolean) = act(null) { engine.setPreciseRetention(on) }

    fun setAllowOlder(on: Boolean) = act(null) { engine.setAllowOlder(on) }

    fun setManualPlace(lat: Double, lon: Double, label: String?) =
        act(null) { engine.setPlace(PlaceInput(lat, lon, label?.ifBlank { null }, "manual")) }

    fun useSavedPlace(p: SavedPlaceView) = act(null) { engine.setPlace(PlaceInput(p.lat, p.lon, p.label, "saved_place")) }

    fun savePlace(label: String) = act(null) {
        val place = _view.value?.place ?: throw SquibException.Rejected("Choose a place first.")
        engine.addSavedPlace(label, place.lat, place.lon, System.currentTimeMillis())
    }

    fun deletePlace(id: String) = act(null) { engine.deleteSavedPlace(id) }

    /** Uses foreground location once (caller has requested permission). */
    fun useMyLocation() = act("Getting location…") {
        val fix = withContext(Dispatchers.IO) { sensors.currentFix() }
            ?: throw SquibException.Rejected("Location unavailable. Enter coordinates or choose a saved place.")
        engine.setPlace(PlaceInput(fix.lat, fix.lon, null, "gps"))
        pushDevice(fix, null)
    }

    /** Reads the phone barometer (and keeps the last location heights). */
    fun readPhoneSensors() = act("Reading phone sensors…") {
        val p = withContext(Dispatchers.IO) { sensors.readBarometer() }
            ?: throw SquibException.Rejected("This phone has no barometer, or it did not report.")
        pushDevice(lastFix, p)
    }

    private var lastFix: DeviceSensors.Fix? = null
    private var lastPressure: DeviceSensors.Pressure? = null

    private fun pushDevice(fix: DeviceSensors.Fix?, p: DeviceSensors.Pressure?) {
        if (fix != null) lastFix = fix
        if (p != null) lastPressure = p
        val f = lastFix
        val pr = lastPressure
        engine.setDeviceReadings(
            DeviceReadings(
                barometerHpa = pr?.hpa,
                barometerUtcMs = pr?.utcMs,
                ellipsoidM = f?.ellipsoidM,
                ellipsoidAccuracyM = f?.ellipsoidAccuracyM,
                mslM = f?.mslM,
                mslAccuracyM = f?.mslAccuracyM,
                locationUtcMs = f?.utcMs,
            ),
        )
    }

    /** Runs the core's bounded refresh plan to completion. */
    fun refresh() = act("Updating weather…") {
        var step = engine.conditionsRefresh(System.currentTimeMillis())
        var pending: List<HttpRequestFfi> = step.requests
        while (!step.done) {
            if (pending.isEmpty()) break
            val responses = withContext(Dispatchers.IO) {
                pending.map { r -> async { WeatherTransport.execute(r) } }.awaitAll()
            }
            pending = emptyList()
            for (resp in responses) {
                step = engine.conditionsResponse(resp, System.currentTimeMillis())
                pending = pending + step.requests
                if (step.done) break
            }
        }
    }

    fun setOverride(field: String, valueSi: Double, entered: Double, unit: String) =
        act(null) { engine.setOverride(field, valueSi, entered, unit, System.currentTimeMillis()) }

    fun setCalmOverride(calm: Boolean) = act(null) { engine.setDirectionStateOverride(calm, System.currentTimeMillis()) }

    fun clearOverride(field: String) = act(null) { engine.clearOverride(field) }

    fun clearCache() = act(null) {
        val n = engine.clearWeatherCache()
        _message.value = "Cleared $n cached weather documents. Your runs and their conditions are unchanged."
    }

    override fun onCleared() {
        worker.close()
    }
}

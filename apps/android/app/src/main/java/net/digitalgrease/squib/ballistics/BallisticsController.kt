package net.digitalgrease.squib.ballistics

import android.app.Application
import android.content.Context
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import java.util.concurrent.Executors
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import net.digitalgrease.squib.SquibApp
import net.digitalgrease.squib.core.AirChoice
import net.digitalgrease.squib.core.AirView
import net.digitalgrease.squib.core.CardRequest
import net.digitalgrease.squib.core.CardView
import net.digitalgrease.squib.core.EquipmentView
import net.digitalgrease.squib.core.LoadInput
import net.digitalgrease.squib.core.LoadView
import net.digitalgrease.squib.core.SquibException

/**
 * Ballistics screen state (plan 6.4). Engine calls run on one worker thread. The screen
 * keeps what the shooter typed as text; the request is built in SI only when solving.
 */
class BallisticsController(app: Application) : AndroidViewModel(app) {
    private val engine = (app as SquibApp).engine
    private val worker = Executors.newSingleThreadExecutor { Thread(it, "squib-ballistics") }.asCoroutineDispatcher()
    private val prefs = app.getSharedPreferences("ballistics_ui", Context.MODE_PRIVATE)

    private val _loads = MutableStateFlow<List<LoadView>>(emptyList())
    val loads: StateFlow<List<LoadView>> = _loads.asStateFlow()
    private val _equipment = MutableStateFlow<List<EquipmentView>>(emptyList())
    val equipment: StateFlow<List<EquipmentView>> = _equipment.asStateFlow()
    private val _air = MutableStateFlow<AirView?>(null)
    val air: StateFlow<AirView?> = _air.asStateFlow()
    private val _result = MutableStateFlow<CardView?>(null)
    /** The last solved card, saved or not. */
    val result: StateFlow<CardView?> = _result.asStateFlow()
    private val _cards = MutableStateFlow<List<CardView>>(emptyList())
    val cards: StateFlow<List<CardView>> = _cards.asStateFlow()
    private val _message = MutableStateFlow<String?>(null)
    val message: StateFlow<String?> = _message.asStateFlow()
    private val _busy = MutableStateFlow(false)
    val busy: StateFlow<Boolean> = _busy.asStateFlow()
    private val _mil = MutableStateFlow(prefs.getBoolean("mil", true))
    /** Corrections in mil (true) or MOA. */
    val mil: StateFlow<Boolean> = _mil.asStateFlow()

    init {
        reload()
    }

    fun setMil(mil: Boolean) {
        _mil.value = mil
        prefs.edit().putBoolean("mil", mil).apply()
    }

    fun clearMessage() {
        _message.value = null
    }

    fun clearResult() {
        _result.value = null
    }

    fun reload() {
        viewModelScope.launch(worker) {
            _loads.value = engine.listLoads()
            _equipment.value = engine.listEquipment()
            _air.value = engine.ballisticsAir(System.currentTimeMillis())
            _cards.value = engine.listCards(50u)
        }
    }

    private fun act(f: () -> Unit) {
        viewModelScope.launch(worker) {
            _busy.value = true
            try {
                f()
            } catch (e: SquibException) {
                _message.value = e.message?.replaceFirstChar { it.uppercase() }
            } finally {
                _busy.value = false
                _loads.value = engine.listLoads()
                _cards.value = engine.listCards(50u)
            }
        }
    }

    fun saveLoad(id: String?, input: LoadInput, onDone: () -> Unit) = act {
        val v = engine.saveLoad(id, input, System.currentTimeMillis())
        engine.setActiveLoad(v.id)
        _result.value = null
        viewModelScope.launch { onDone() }
    }

    fun selectLoad(id: String) = act {
        engine.setActiveLoad(id)
        _result.value = null
    }

    fun archiveLoad(id: String) = act {
        engine.archiveLoad(id)
        _result.value = null
    }

    fun solve(req: CardRequest) = act {
        _result.value = engine.solveCard(req, System.currentTimeMillis())
    }

    fun save(req: CardRequest, title: String) = act {
        _result.value = engine.saveCard(req, title, System.currentTimeMillis())
        _message.value = "Range card saved with its inputs and the method used."
    }

    fun deleteCard(id: String) = act {
        engine.deleteCard(id)
        if (_result.value?.id == id) _result.value = null
    }

    fun show(card: CardView) {
        _result.value = card
    }

    companion object {
        fun request(loadId: String, air: AirChoice, windMps: Double, windFromDeg: Double, angleDeg: Double, ranges: List<Double>) =
            CardRequest(loadId, air, windMps, windFromDeg, angleDeg, ranges)
    }
}

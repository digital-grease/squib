package net.digitalgrease.squib.data

import android.app.Application
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.Uri
import android.os.Build
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import java.io.File
import java.util.Locale
import java.util.TimeZone
import java.util.UUID
import java.util.concurrent.Executors
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import net.digitalgrease.squib.BuildConfig
import net.digitalgrease.squib.SquibApp
import net.digitalgrease.squib.core.AnalyticsFilterFfi
import net.digitalgrease.squib.core.AnalyticsView
import net.digitalgrease.squib.core.ChecklistView
import net.digitalgrease.squib.core.CoachView
import net.digitalgrease.squib.core.PlanItemInput
import net.digitalgrease.squib.core.PlanSummaryView
import net.digitalgrease.squib.core.PlanView
import net.digitalgrease.squib.core.DrillInput
import net.digitalgrease.squib.core.DrillView
import net.digitalgrease.squib.core.ImportView
import net.digitalgrease.squib.core.RoundTotalsView
import net.digitalgrease.squib.core.ScoringProfileView
import net.digitalgrease.squib.core.ShooterView
import net.digitalgrease.squib.core.SquibException

/**
 * M3 data operations: drills, profiles, analytics, manual entry, attachments, backup,
 * import, CSV, deletion, and the bug report. Engine calls run on one worker thread.
 */
class DataController(private val app: Application) : AndroidViewModel(app) {
    private val engine = (app as SquibApp).engine
    private val worker = Executors.newSingleThreadExecutor { Thread(it, "squib-data") }.asCoroutineDispatcher()
    val attachmentRoot: File = File(app.filesDir, "attachments").also { it.mkdirs() }

    private val _drills = MutableStateFlow<List<DrillView>>(emptyList())
    val drills: StateFlow<List<DrillView>> = _drills.asStateFlow()
    private val _shooters = MutableStateFlow<List<ShooterView>>(emptyList())
    val shooters: StateFlow<List<ShooterView>> = _shooters.asStateFlow()
    private val _analytics = MutableStateFlow<AnalyticsView?>(null)
    val analytics: StateFlow<AnalyticsView?> = _analytics.asStateFlow()
    private val _rounds = MutableStateFlow<RoundTotalsView?>(null)
    val rounds: StateFlow<RoundTotalsView?> = _rounds.asStateFlow()
    private val _message = MutableStateFlow<String?>(null)
    val message: StateFlow<String?> = _message.asStateFlow()
    private val _pendingImport = MutableStateFlow<Pair<File, ImportView>?>(null)
    val pendingImport: StateFlow<Pair<File, ImportView>?> = _pendingImport.asStateFlow()
    private val _plans = MutableStateFlow<List<PlanSummaryView>>(emptyList())
    val plans: StateFlow<List<PlanSummaryView>> = _plans.asStateFlow()
    private val _coach = MutableStateFlow<CoachView?>(null)
    val coach: StateFlow<CoachView?> = _coach.asStateFlow()
    private val _filter = MutableStateFlow(AnalyticsFilterFfi(null, null, null, true, false, false, false))
    val filter: StateFlow<AnalyticsFilterFfi> = _filter.asStateFlow()

    val profiles: List<ScoringProfileView> = engine.scoringProfiles()

    init {
        reload()
    }

    fun clearMessage() {
        _message.value = null
    }

    private fun tz(): Int = TimeZone.getDefault().getOffset(System.currentTimeMillis()) / 60_000

    fun reload() {
        viewModelScope.launch(worker) { refresh() }
    }

    private fun refresh() {
        _drills.value = engine.listDrills()
        _shooters.value = engine.listShooters()
        _rounds.value = engine.roundTotals()
        _analytics.value = runCatching { engine.analytics(_filter.value) }.getOrNull()
        _plans.value = runCatching { engine.listPlans() }.getOrDefault(emptyList())
        _coach.value = engine.coach()
    }

    private fun act(f: () -> Unit) {
        viewModelScope.launch(worker) {
            try {
                f()
            } catch (e: SquibException) {
                _message.value = e.message
            } catch (e: Exception) {
                _message.value = "Failed: ${e.message}"
            }
            refresh()
        }
    }

    fun setFilter(f: AnalyticsFilterFfi) {
        _filter.value = f
        reload()
    }

    // Drills
    fun createDrill(d: DrillInput) = act { engine.createDrill(d, System.currentTimeMillis()) }
    fun editDrill(id: String, d: DrillInput) = act { engine.editDrill(id, d, System.currentTimeMillis()) }
    fun archiveDrill(id: String) = act { engine.archiveDrill(id) }
    suspend fun drillFile(d: DrillView): ByteArray = withContext(worker) { engine.drillFile(d.drillId, d.version) }
    fun importDrill(uri: Uri) = act {
        val bytes = app.contentResolver.openInputStream(uri)?.use { s -> s.readNBytesCompat(16 * 1024 + 1) } ?: ByteArray(0)
        engine.importDrillFile(bytes, System.currentTimeMillis())
        _message.value = "Drill imported."
    }

    // Profiles
    fun addShooter(name: String) = act { engine.addShooter(name, System.currentTimeMillis()) }
    fun setActiveShooter(id: String) = act { engine.setActiveShooter(id) }

    // Manual entry (A20)
    fun addManual(label: String, precisionMs: Int, times: String, drill: DrillView?, planItemId: String? = null, onDone: (String) -> Unit) = act {
        val id = engine.addManualRun(
            label, precisionMs.toUInt(), times, drill?.drillId, drill?.version,
            System.currentTimeMillis(), tz(), "${BuildConfig.VERSION_NAME}+${BuildConfig.VERSION_CODE}",
        )
        planItemId?.let { engine.linkRunToItem(it, id) }
        viewModelScope.launch { onDone(id) }
    }

    // Coach rotation (M4)
    fun setCoach(enabled: Boolean, squad: List<String>) = act { engine.setCoach(enabled, squad) }
    fun advanceShooter() = act { engine.advanceShooter() }

    // Day plans (M4)
    private fun localMinutes(): UInt {
        val c = java.util.Calendar.getInstance()
        return (c.get(java.util.Calendar.HOUR_OF_DAY) * 60 + c.get(java.util.Calendar.MINUTE)).toUInt()
    }

    fun today(): String = java.text.SimpleDateFormat("yyyy-MM-dd", Locale.US).format(java.util.Date())

    suspend fun loadPlan(id: String): PlanView? = withContext(worker) { runCatching { engine.loadPlan(id, localMinutes()) }.getOrNull() }

    suspend fun drillVersion(id: String, version: UInt): DrillView? =
        withContext(worker) { runCatching { engine.drillVersion(id, version) }.getOrNull() }

    fun createPlan(title: String, date: String, kind: String, onDone: (String) -> Unit) = act {
        val id = engine.createPlan(title, date, kind, true, System.currentTimeMillis())
        viewModelScope.launch { onDone(id) }
    }

    /** Plan edits run on the worker, then `after` runs on the main thread (usually a reload). */
    fun planEdit(after: () -> Unit, f: () -> Unit) = act {
        f()
        viewModelScope.launch { after() }
    }

    fun addPlanItem(planId: String, input: PlanItemInput, after: () -> Unit) = planEdit(after) { engine.addPlanItem(planId, input) }
    fun setPlanNotes(planId: String, notes: String, after: () -> Unit) = planEdit(after) { engine.setPlanNotes(planId, notes) }
    fun setItemNotes(id: String, notes: String, after: () -> Unit) = planEdit(after) { engine.setItemNotes(id, notes) }
    fun setItemSkipped(id: String, skipped: Boolean, after: () -> Unit) = planEdit(after) { engine.setItemSkipped(id, skipped) }
    fun movePlanItem(id: String, up: Boolean, after: () -> Unit) = planEdit(after) { engine.movePlanItem(id, up) }
    fun deletePlanItem(id: String, after: () -> Unit) = planEdit(after) { engine.deletePlanItem(id) }
    fun archivePlan(id: String, after: () -> Unit) = planEdit(after) { engine.archivePlan(id) }
    fun addChecklistItem(planId: String?, text: String, after: () -> Unit) = planEdit(after) { engine.addChecklistItem(planId, text) }
    fun setChecked(id: String, checked: Boolean, after: () -> Unit) = planEdit(after) { engine.setChecklistChecked(id, checked) }
    fun deleteChecklistItem(id: String, after: () -> Unit) = planEdit(after) { engine.deleteChecklistItem(id) }
    suspend fun checklistTemplate(): List<ChecklistView> = withContext(worker) { runCatching { engine.checklistTemplate() }.getOrDefault(emptyList()) }

    // Per-run review data
    suspend fun score(runId: String, profileId: String?): net.digitalgrease.squib.core.ScoreView? =
        withContext(worker) { runCatching { engine.runScore(runId, profileId) }.getOrNull() }

    suspend fun saveScore(runId: String, profileId: String, counts: Map<String, Int>, complete: Boolean): Result<net.digitalgrease.squib.core.ScoreView> =
        withContext(worker) {
            runCatching {
                engine.setScore(
                    runId, profileId, counts.map { net.digitalgrease.squib.core.ScoreCount(it.key, it.value.toUInt()) },
                    complete, null, "", System.currentTimeMillis(),
                )
            }
        }

    suspend fun rounds(runId: String): net.digitalgrease.squib.core.RoundsView? =
        withContext(worker) { runCatching { engine.runRounds(runId) }.getOrNull() }

    suspend fun confirmRounds(runId: String, n: Int): net.digitalgrease.squib.core.RoundsView? =
        withContext(worker) { runCatching { engine.confirmRounds(runId, n.toUInt(), System.currentTimeMillis()) }.getOrNull() }

    suspend fun attachments(runId: String): List<net.digitalgrease.squib.core.AttachmentView> =
        withContext(worker) { engine.runAttachments(runId) }

    suspend fun shareJson(runIds: List<String>): String = withContext(worker) { engine.shareResults(runIds) }

    // Rounds
    fun setCost(minor: Long?, currency: String?) = act { engine.setRoundCost(minor, currency) }

    /** Copy a picked photo into app storage, re-encoded as JPEG so EXIF/GPS metadata is dropped. */
    fun addPhoto(runId: String, uri: Uri, onDone: () -> Unit) = act {
        val bmp: Bitmap = app.contentResolver.openInputStream(uri)?.use { BitmapFactory.decodeStream(it) }
            ?: throw IllegalStateException("could not read image")
        val rel = "photos/${UUID.randomUUID()}.jpg"
        val dest = File(attachmentRoot, rel).also { it.parentFile?.mkdirs() }
        val tmp = File(dest.path + ".partial")
        tmp.outputStream().use { bmp.compress(Bitmap.CompressFormat.JPEG, 90, it) }
        tmp.renameTo(dest)
        engine.registerAttachment(runId, attachmentRoot.absolutePath, rel, "image/jpeg", true, System.currentTimeMillis())
        viewModelScope.launch { onDone() }
    }

    // Backup / import / CSV (A17)
    fun exportBackup(target: Uri, includePhotos: Boolean) = act {
        val tmp = File(app.cacheDir, "squib-backup.zip")
        val v = engine.exportBackup(tmp.absolutePath, if (includePhotos) attachmentRoot.absolutePath else null, BuildConfig.VERSION_NAME, System.currentTimeMillis())
        app.contentResolver.openOutputStream(target)?.use { out -> tmp.inputStream().use { it.copyTo(out) } }
        tmp.delete()
        _message.value = "Backup saved: ${v.runs} runs, ${v.attachments} photos, ${v.bytes / 1024u} KB. " +
            "It is not encrypted" + (if (v.containsLocation) " and contains saved places or location detail." else ".")
    }

    fun exportCsv(target: Uri) = act {
        val csv = engine.exportCsv()
        app.contentResolver.openOutputStream(target)?.use { it.write(csv.toByteArray()) }
        _message.value = "CSV exported."
    }

    fun previewImport(source: Uri) = act {
        val tmp = File(app.cacheDir, "squib-import.zip")
        app.contentResolver.openInputStream(source)?.use { input ->
            tmp.outputStream().use { out ->
                val copied = input.copyToLimited(out, 512L * 1024 * 1024)
                if (!copied) throw IllegalStateException("backup is larger than 512 MB")
            }
        }
        val v = engine.previewImport(tmp.absolutePath, app.cacheDir.absolutePath)
        _pendingImport.value = tmp to v
    }

    fun confirmImport() = act {
        val (file, _) = _pendingImport.value ?: return@act
        _pendingImport.value = null
        val v = engine.importBackup(file.absolutePath, app.cacheDir.absolutePath, attachmentRoot.absolutePath)
        file.delete()
        _message.value = "Imported ${v.runsNew} new runs (${v.runsAlreadyPresent} already present), ${v.attachmentsRestored} photos."
    }

    fun cancelImport() {
        _pendingImport.value?.first?.delete()
        _pendingImport.value = null
    }

    // Deletion
    fun deleteRun(runId: String, onDone: () -> Unit) = act {
        val d = engine.deleteRun(runId)
        d.attachmentPaths.forEach { deleteAttachmentFile(it) }
        viewModelScope.launch { onDone() }
    }

    fun deleteAll() = act {
        val d = engine.deleteAllHistory()
        d.attachmentPaths.forEach { deleteAttachmentFile(it) }
        _message.value = "Deleted ${d.runs} runs and all saved places, drills, and cached weather. Copies you exported or OS backups are not affected."
    }

    /** Delete an app-owned attachment, refusing any path that resolves outside the root. */
    private fun deleteAttachmentFile(relative: String) {
        val root = attachmentRoot.canonicalFile
        val f = File(root, relative).canonicalFile
        if (f.path.startsWith(root.path + File.separator)) f.delete()
    }

    // Bug report
    suspend fun issueReport(): IssueReport.Result = withContext(worker) {
        val device = IssueReport.formatDevice(Build.MANUFACTURER, Build.MODEL)
        val android = "${Build.VERSION.RELEASE} (API ${Build.VERSION.SDK_INT})"
        val version = "${BuildConfig.VERSION_NAME}+${BuildConfig.VERSION_CODE}"
        IssueReport.build(device, android, version, engine.diagnosticReport(device, android, version))
    }

    suspend fun reportText(): String = withContext(worker) {
        val device = IssueReport.formatDevice(Build.MANUFACTURER, Build.MODEL)
        engine.diagnosticReport(device, "${Build.VERSION.RELEASE} (API ${Build.VERSION.SDK_INT})", BuildConfig.VERSION_NAME)
    }

    override fun onCleared() {
        worker.close()
    }
}

private fun java.io.InputStream.readNBytesCompat(n: Int): ByteArray {
    val out = java.io.ByteArrayOutputStream()
    val buf = ByteArray(8192)
    var total = 0
    while (total < n) {
        val r = read(buf, 0, minOf(buf.size, n - total))
        if (r < 0) break
        out.write(buf, 0, r)
        total += r
    }
    return out.toByteArray()
}

/** Copy at most `limit` bytes; returns false if the source was larger. */
private fun java.io.InputStream.copyToLimited(out: java.io.OutputStream, limit: Long): Boolean {
    val buf = ByteArray(64 * 1024)
    var total = 0L
    while (true) {
        val r = read(buf)
        if (r < 0) return true
        total += r
        if (total > limit) return false
        out.write(buf, 0, r)
    }
}

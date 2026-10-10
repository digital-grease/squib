package net.digitalgrease.squib

import android.Manifest
import android.content.pm.PackageManager
import android.os.Bundle
import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.activity.viewModels
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import net.digitalgrease.squib.conditions.ConditionsController
import net.digitalgrease.squib.conditions.ConditionsScreen
import net.digitalgrease.squib.core.SquibController
import net.digitalgrease.squib.data.DataController
import net.digitalgrease.squib.data.IssueReport
import net.digitalgrease.squib.ui.DataScreen
import net.digitalgrease.squib.ui.ChecklistTemplateScreen
import net.digitalgrease.squib.ui.PlanScreen
import net.digitalgrease.squib.ui.PracticeScreen
import android.content.Intent
import androidx.activity.result.PickVisualMediaRequest
import androidx.lifecycle.lifecycleScope
import kotlinx.coroutines.launch
import net.digitalgrease.squib.ui.HistoryScreen
import net.digitalgrease.squib.ui.ReviewScreen
import net.digitalgrease.squib.ui.SetupScreen
import net.digitalgrease.squib.ui.SquibTheme
import net.digitalgrease.squib.ui.TimerScreen

class MainActivity : ComponentActivity() {
    private val controller: SquibController by viewModels()
    private val conditions: ConditionsController by viewModels()
    private var afterLocation: (() -> Unit)? = null
    private val locationPermission = registerForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { r ->
        val next = afterLocation
        afterLocation = null
        if (r.values.any { it }) next?.invoke() else locationDenied.value = true
    }
    private val locationDenied = mutableStateOf(false)

    /** Location is requested only when the user asks to use it; manual places always work. */
    private fun withLocation(then: () -> Unit) {
        if (checkSelfPermission(Manifest.permission.ACCESS_COARSE_LOCATION) == PackageManager.PERMISSION_GRANTED ||
            checkSelfPermission(Manifest.permission.ACCESS_FINE_LOCATION) == PackageManager.PERMISSION_GRANTED
        ) {
            then()
        } else {
            afterLocation = then
            locationPermission.launch(arrayOf(Manifest.permission.ACCESS_FINE_LOCATION, Manifest.permission.ACCESS_COARSE_LOCATION))
        }
    }
    private var afterPermission: (() -> Unit)? = null
    private val permission = registerForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        val next = afterPermission
        afterPermission = null
        if (granted) next?.invoke() else deniedMessage.value = true
        controller.refreshPreflight()
    }
    private val deniedMessage = mutableStateOf(false)
    private var afterCamera: (() -> Unit)? = null
    private val cameraPermission = registerForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        val next = afterCamera
        afterCamera = null
        if (granted) next?.invoke() else cameraDenied.value = true
    }
    private val cameraDenied = mutableStateOf(false)

    /** Camera is requested only when a run with video is armed. */
    private fun withCamera(then: () -> Unit) {
        if (checkSelfPermission(Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED) {
            then()
        } else {
            afterCamera = then
            cameraPermission.launch(Manifest.permission.CAMERA)
        }
    }
    private val data: DataController by viewModels()
    private var photoRun: String? = null
    private var backupWithPhotos = true
    private var backupWithAudio = false
    private var diagnosticRun: String? = null
    private val createDiagnostic = registerForActivityResult(ActivityResultContracts.CreateDocument("application/zip")) { uri ->
        val run = diagnosticRun
        diagnosticRun = null
        // Consent was confirmed in the preview before the picker opened.
        if (uri != null && run != null) data.exportDiagnostic(run, uri, consent = true)
    }
    private val createBackup = registerForActivityResult(ActivityResultContracts.CreateDocument("application/zip")) { uri ->
        uri?.let { data.exportBackup(it, backupWithPhotos, backupWithAudio) }
    }
    private val createCsv = registerForActivityResult(ActivityResultContracts.CreateDocument("text/csv")) { uri ->
        uri?.let { data.exportCsv(it) }
    }
    private val openBackup = registerForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
        uri?.let { data.previewImport(it) }
    }
    private val openDrill = registerForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
        uri?.let { data.importDrill(it) }
    }
    private val pickPhoto = registerForActivityResult(ActivityResultContracts.PickVisualMedia()) { uri ->
        val run = photoRun
        if (uri != null && run != null) data.addPhoto(run, uri) { photoVersion.value++ }
    }
    private val photoVersion = mutableStateOf(0)

    private fun shareText(subject: String, text: String) {
        val send = Intent(Intent.ACTION_SEND).setType("text/plain").putExtra(Intent.EXTRA_SUBJECT, subject).putExtra(Intent.EXTRA_TEXT, text)
        startActivity(Intent.createChooser(send, subject))
    }

    private fun copy(label: String, text: String) {
        (getSystemService(CLIPBOARD_SERVICE) as android.content.ClipboardManager)
            .setPrimaryClip(android.content.ClipData.newPlainText(label, text))
    }

    /** Opens the pre-filled GitHub issue form; the user submits it. Clipboard is the fallback. */
    private fun reportProblem() {
        lifecycleScope.launch {
            val r = data.issueReport()
            if (r is IssueReport.Result.Truncated) copy("Squib report", r.fullReport)
            try {
                startActivity(Intent(Intent.ACTION_VIEW, android.net.Uri.parse(r.url)))
                if (r is IssueReport.Result.Truncated) {
                    android.widget.Toast.makeText(this@MainActivity, "Report was long: the full text is on your clipboard to paste.", android.widget.Toast.LENGTH_LONG).show()
                }
            } catch (e: android.content.ActivityNotFoundException) {
                copy("Squib report", data.reportText())
                android.widget.Toast.makeText(this@MainActivity, "No browser found. The report was copied to your clipboard.", android.widget.Toast.LENGTH_LONG).show()
            }
        }
    }

    /** Microphone is requested only at the point of use, never for par-only (A02). */
    private fun withMic(then: () -> Unit) {
        if (checkSelfPermission(Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED) {
            then()
        } else {
            afterPermission = then
            permission.launch(Manifest.permission.RECORD_AUDIO)
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            SquibTheme {
                val keepOn by controller.keepScreenOn.collectAsState()
                LaunchedEffect(keepOn) {
                    if (keepOn) window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
                    else window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
                }
                var tab by rememberSaveable { mutableStateOf("timer") }
                var reviewRun by rememberSaveable { mutableStateOf<String?>(null) }
                var openPlan by rememberSaveable { mutableStateOf<String?>(null) }
                val msg by controller.message.collectAsState()
                val condMsg by conditions.message.collectAsState()
                val dataMsg by data.message.collectAsState()
                if (dataMsg != null) {
                    AlertDialog(
                        onDismissRequest = data::clearMessage,
                        confirmButton = { TextButton(onClick = data::clearMessage) { Text("OK") } },
                        text = { Text(dataMsg!!) },
                    )
                }
                val locDenied by locationDenied
                val denied by deniedMessage
                Scaffold(
                    bottomBar = {
                        NavigationBar {
                            NavigationBarItem(
                                selected = tab == "timer", onClick = { tab = "timer"; reviewRun = null },
                                icon = { Text("⏱") }, label = { Text("Timer") }, modifier = Modifier.testTag("tab_timer"),
                            )
                            NavigationBarItem(
                                selected = tab in setOf("practice", "plan", "checklist"),
                                onClick = { tab = if (openPlan != null) "plan" else "practice"; reviewRun = null },
                                icon = { Text("◎") }, label = { Text("Practice") }, modifier = Modifier.testTag("tab_practice"),
                            )
                            NavigationBarItem(
                                selected = tab == "conditions", onClick = { tab = "conditions"; reviewRun = null },
                                icon = { Text("☁") }, label = { Text("Conditions") }, modifier = Modifier.testTag("tab_conditions"),
                            )
                            NavigationBarItem(
                                selected = tab == "history", onClick = { tab = "history"; reviewRun = null },
                                icon = { Text("☰") }, label = { Text("History") }, modifier = Modifier.testTag("tab_history"),
                            )
                        }
                    },
                ) { pad ->
                    Box(Modifier.padding(pad).fillMaxSize()) {
                        val run = reviewRun
                        when {
                            run != null -> {
                                val pv by photoVersion
                                androidx.compose.runtime.key(run, pv) {
                                    ReviewScreen(controller, data, run, onBack = { reviewRun = null; controller.refreshHistory(); data.reload() }, onRepeat = {
                                        reviewRun = null; tab = "timer"; controller.repeat()
                                    }, onAddPhoto = {
                                        photoRun = run
                                        pickPhoto.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageOnly))
                                    }, onShare = { json -> shareText("Squib results", json) }, onExportDiagnostic = {
                                        diagnosticRun = run
                                        createDiagnostic.launch("squib-diagnostic.zip")
                                    })
                                }
                            }
                            tab == "data" -> DataScreen(
                                data,
                                onBack = { tab = "history" },
                                onExportBackup = { photos, audio ->
                                    backupWithPhotos = photos; backupWithAudio = audio; createBackup.launch("squib-backup.zip")
                                },
                                onImport = { openBackup.launch(arrayOf("application/zip", "application/octet-stream")) },
                                onExportCsv = { createCsv.launch("squib-runs.csv") },
                                onReportProblem = ::reportProblem,
                            )
                            tab == "practice" -> PracticeScreen(
                                data,
                                onStartDrill = { d -> controller.startDrill(d); tab = "timer" },
                                onShareDrill = { d, bytes -> shareText("Squib drill: ${d.input.title}", String(bytes)) },
                                onImportDrill = { openDrill.launch(arrayOf("application/json", "text/plain", "application/octet-stream")) },
                                onOpenRun = { reviewRun = it },
                                onOpenPlan = { openPlan = it; tab = "plan" },
                                onTemplate = { tab = "checklist" },
                            )
                            tab == "plan" && openPlan != null -> PlanScreen(
                                data, openPlan!!,
                                onBack = { openPlan = null; tab = "practice"; data.reload() },
                                onStartDrill = { d, item, title, strings -> controller.startDrill(d, item, title, strings); tab = "timer" },
                                onOpenRun = { reviewRun = it },
                            )
                            tab == "checklist" -> ChecklistTemplateScreen(data, onBack = { tab = "practice" })
                            tab == "setup" -> SetupScreen(controller, ::withMic, onBack = { tab = "timer" })
                            tab == "history" -> HistoryScreen(controller, data, onOpen = { reviewRun = it }, onData = { tab = "data" })
                            tab == "conditions" -> ConditionsScreen(conditions, ::withLocation)
                            else -> TimerScreen(controller, data, ::withMic, ::withCamera, onReview = { reviewRun = it }, onSetup = { tab = "setup" })
                        }
                    }
                }
                if (msg != null) {
                    AlertDialog(
                        onDismissRequest = controller::clearMessage,
                        confirmButton = { TextButton(onClick = controller::clearMessage) { Text("OK") } },
                        text = { Text(msg!!) },
                    )
                }
                if (condMsg != null) {
                    AlertDialog(
                        onDismissRequest = conditions::clearMessage,
                        confirmButton = { TextButton(onClick = conditions::clearMessage) { Text("OK") } },
                        text = { Text(condMsg!!) },
                    )
                }
                if (locDenied) {
                    AlertDialog(
                        onDismissRequest = { locationDenied.value = false },
                        confirmButton = { TextButton(onClick = { locationDenied.value = false }) { Text("OK") } },
                        text = { Text("Location permission was not granted. You can enter coordinates or use a saved place instead.") },
                    )
                }
                val camDenied by cameraDenied
                if (camDenied) {
                    AlertDialog(
                        onDismissRequest = { cameraDenied.value = false },
                        confirmButton = { TextButton(onClick = { cameraDenied.value = false }) { Text("OK") } },
                        text = { Text("Camera permission was not granted. Turn video off to time runs without it.") },
                    )
                }
                if (denied) {
                    AlertDialog(
                        onDismissRequest = { deniedMessage.value = false },
                        confirmButton = { TextButton(onClick = { deniedMessage.value = false }) { Text("OK") } },
                        text = { Text("Microphone permission was not granted. Par-only timing still works without it.") },
                    )
                }
            }
        }
    }

    override fun onStop() {
        super.onStop()
        // Foreground-only capture (ADR-006): leaving the foreground interrupts a run.
        if (!isChangingConfigurations) controller.onForegroundLost()
    }
}

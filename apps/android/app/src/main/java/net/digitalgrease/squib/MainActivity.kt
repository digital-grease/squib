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
import net.digitalgrease.squib.core.SquibController
import net.digitalgrease.squib.ui.HistoryScreen
import net.digitalgrease.squib.ui.ReviewScreen
import net.digitalgrease.squib.ui.SetupScreen
import net.digitalgrease.squib.ui.SquibTheme
import net.digitalgrease.squib.ui.TimerScreen

class MainActivity : ComponentActivity() {
    private val controller: SquibController by viewModels()
    private var afterPermission: (() -> Unit)? = null
    private val permission = registerForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        val next = afterPermission
        afterPermission = null
        if (granted) next?.invoke() else deniedMessage.value = true
        controller.refreshPreflight()
    }
    private val deniedMessage = mutableStateOf(false)

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
                val msg by controller.message.collectAsState()
                val denied by deniedMessage
                Scaffold(
                    bottomBar = {
                        NavigationBar {
                            NavigationBarItem(
                                selected = tab == "timer", onClick = { tab = "timer"; reviewRun = null },
                                icon = { Text("⏱") }, label = { Text("Timer") }, modifier = Modifier.testTag("tab_timer"),
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
                            run != null -> ReviewScreen(controller, run, onBack = { reviewRun = null }, onRepeat = {
                                reviewRun = null; tab = "timer"; controller.repeat()
                            })
                            tab == "setup" -> SetupScreen(controller, ::withMic, onBack = { tab = "timer" })
                            tab == "history" -> HistoryScreen(controller, onOpen = { reviewRun = it })
                            else -> TimerScreen(controller, ::withMic, onReview = { reviewRun = it }, onSetup = { tab = "setup" })
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

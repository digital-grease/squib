package net.digitalgrease.squib.ui

import android.widget.VideoView
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Card
import androidx.compose.material3.FilterChip
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import java.io.File
import java.util.Locale
import kotlinx.coroutines.delay
import net.digitalgrease.squib.core.VideoClipView
import net.digitalgrease.squib.core.VideoMarker
import net.digitalgrease.squib.data.DataController

private fun markerText(m: VideoMarker): String =
    m.label + when {
        m.approximate && m.edited -> " (edited, approximate)"
        m.edited -> " (edited)"
        m.approximate -> " (approximate)"
        else -> ""
    }

/**
 * Clips recorded with this run. Markers come from the run's own event references (the
 * latest review revision), placed on the clip by the core's labelled clock mapping.
 */
@Composable
fun VideoSection(d: DataController, runId: String) {
    var clips by remember { mutableStateOf<List<VideoClipView>>(emptyList()) }
    LaunchedEffect(runId) { clips = d.videos(runId) }
    clips.forEach { VideoClipCard(d.attachmentRoot, it) }
}

@Composable
private fun VideoClipCard(root: File, clip: VideoClipView) {
    var view by remember { mutableStateOf<VideoView?>(null) }
    var pos by remember { mutableLongStateOf(0L) }
    LaunchedEffect(view) {
        while (view != null) {
            pos = view?.currentPosition?.toLong() ?: 0L
            delay(100)
        }
    }
    // The latest marker reached in the last 1.5 s is shown over the video.
    val current = clip.markers.lastOrNull { pos >= it.fileMs && pos - it.fileMs < 1500 }
    Card(Modifier.fillMaxWidth().padding(vertical = 8.dp).testTag("video_section")) {
        Column(Modifier.padding(12.dp)) {
            Text("Video", fontWeight = FontWeight.Bold)
            Box(Modifier.fillMaxWidth().height(220.dp).background(Color.Black)) {
                AndroidView(
                    factory = { ctx ->
                        VideoView(ctx).apply {
                            setVideoPath(File(root, clip.relativePath).path)
                            setOnPreparedListener { it.isLooping = false; seekTo(1) }
                            view = this
                        }
                    },
                    onRelease = { view = null },
                    modifier = Modifier.fillMaxWidth().height(220.dp),
                )
                current?.let {
                    Text(markerText(it), color = Color.White, fontSize = 20.sp, fontWeight = FontWeight.Black,
                        modifier = Modifier.align(Alignment.TopStart).background(Color(0x99000000)).padding(8.dp).testTag("video_overlay"))
                }
            }
            Row(verticalAlignment = Alignment.CenterVertically) {
                OutlinedButton(onClick = { view?.let { if (it.isPlaying) it.pause() else it.start() } },
                    modifier = Modifier.heightIn(min = 48.dp).padding(end = 8.dp).testTag("video_play")) { Text("Play / pause") }
                Text(String.format(Locale.US, "%.1f s of %.1f s", pos / 1000.0, clip.durationMs / 1000.0), style = MaterialTheme.typography.bodySmall)
            }
            val label = when (clip.mapping) {
                "measured" -> "Aligned (measured)"
                "assumed" -> "Aligned (assumed)"
                else -> "Not aligned"
            }
            Row { Badge(label, warning = clip.mapping != "measured") }
            Text(clip.mappingNote, style = MaterialTheme.typography.bodySmall, modifier = Modifier.testTag("video_mapping"))
            if (clip.markers.isNotEmpty()) {
                Text("Markers are accurate to about ±${clip.uncertaintyMs} ms (one video frame) at best. Tap one to jump just before it.",
                    style = MaterialTheme.typography.bodySmall)
                FlowRow {
                    clip.markers.forEach { m ->
                        FilterChip(
                            selected = current == m,
                            onClick = { view?.let { it.seekTo((m.fileMs - 1000).coerceAtLeast(0).toInt()); it.start() } },
                            label = { Text(markerText(m)) },
                            modifier = Modifier.padding(end = 6.dp).heightIn(min = 48.dp).testTag("marker_${m.kind}"),
                        )
                    }
                }
            }
            if (clip.outsideClip > 0u) Text("${clip.outsideClip} event(s) happened outside the recorded clip.", style = MaterialTheme.typography.bodySmall)
            Text(String.format(Locale.US, "%dx%d, %.0f fps, %.1f MB, no sound.", clip.width.toInt(), clip.height.toInt(), clip.frameRate, clip.bytes / 1e6),
                style = MaterialTheme.typography.bodySmall)
        }
    }
}

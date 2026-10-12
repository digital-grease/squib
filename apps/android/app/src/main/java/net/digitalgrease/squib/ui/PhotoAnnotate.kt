package net.digitalgrease.squib.ui

import android.graphics.BitmapFactory
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.Image
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.FilterChip
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import java.util.Locale
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import net.digitalgrease.squib.core.AttachmentView
import net.digitalgrease.squib.core.MarkView
import net.digitalgrease.squib.core.PhotoAnnotationView
import net.digitalgrease.squib.data.DataController

private val KINDS = listOf("hit" to "Hit", "miss" to "Miss", "note" to "Note")

/** Where a mark is, in words, for screen readers and the list under the photo. */
private fun describe(m: MarkView): String {
    val kind = KINDS.first { it.first == m.kind }.second
    return String.format(Locale.US, "%s at %.0f%% across, %.0f%% down", kind, m.x * 100, m.y * 100) +
        (m.label?.let { ", \"$it\"" } ?: "")
}

/**
 * Manual marks on a target photo (plan phase 4.2). Tap the photo to add a mark of the chosen
 * kind; tap a mark to select it. Marks are positions on the photo (fractions of its size),
 * saved as a new version linked to the run's review and score revisions. Nothing is detected
 * automatically.
 */
@Composable
fun PhotoAnnotate(d: DataController, photo: AttachmentView, onClose: () -> Unit) {
    var image by remember { mutableStateOf<ImageBitmap?>(null) }
    var loaded by remember { mutableStateOf<PhotoAnnotationView?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    val marks = remember { mutableStateListOf<MarkView>() }
    var note by remember { mutableStateOf("") }
    var kind by remember { mutableStateOf("hit") }
    var selected by remember { mutableStateOf<Int?>(null) }
    val scope = rememberCoroutineScope()

    LaunchedEffect(photo.id) {
        image = withContext(Dispatchers.IO) {
            d.attachmentFile(photo.relativePath)?.let { f ->
                val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
                BitmapFactory.decodeFile(f.path, bounds)
                var sample = 1
                while (maxOf(bounds.outWidth, bounds.outHeight) / sample > 1600) sample *= 2
                BitmapFactory.decodeFile(f.path, BitmapFactory.Options().apply { inSampleSize = sample })?.asImageBitmap()
            }
        }
        d.photoAnnotation(photo.id).onSuccess { a ->
            loaded = a
            marks.clear()
            marks.addAll(a.marks)
            note = a.note
        }.onFailure { error = it.message }
    }

    Column(Modifier.fillMaxWidth().padding(16.dp).verticalScroll(rememberScrollState()).testTag("annotate")) {
        BackButton("Back", "Back to review", onClose)
        Text("Target photo", style = MaterialTheme.typography.headlineSmall)
        val img = image
        if (img == null) {
            Text("The photo could not be shown.")
        } else {
            Box(
                Modifier.fillMaxWidth().aspectRatio(img.width.toFloat() / img.height)
                    .semantics { contentDescription = "Target photo with ${marks.size} marks. Tap to add a ${kind} mark." }
                    .testTag("annotate_photo"),
            ) {
                Image(img, contentDescription = null, contentScale = ContentScale.FillBounds, modifier = Modifier.fillMaxSize())
                val accent = MaterialTheme.colorScheme.primary
                Canvas(
                    Modifier.fillMaxSize().pointerInput(kind) {
                        detectTapGestures { p ->
                            val (w, h) = size.width.toFloat() to size.height.toFloat()
                            val near = marks.indexOfFirst { m ->
                                val dx = m.x.toFloat() * w - p.x
                                val dy = m.y.toFloat() * h - p.y
                                dx * dx + dy * dy < (28.dp.toPx()) * (28.dp.toPx())
                            }
                            if (near >= 0) {
                                selected = near
                            } else {
                                marks += MarkView((p.x / w).toDouble().coerceIn(0.0, 1.0), (p.y / h).toDouble().coerceIn(0.0, 1.0), kind, null)
                                selected = marks.lastIndex
                            }
                        }
                    },
                ) {
                    marks.forEachIndexed { i, m ->
                        val c = Offset(m.x.toFloat() * size.width, m.y.toFloat() * size.height)
                        val r = 14.dp.toPx()
                        // Shapes as well as colours: hit = filled circle, miss = cross, note = ring.
                        val outline = if (i == selected) Color.White else Color.Black
                        when (m.kind) {
                            "hit" -> {
                                drawCircle(outline, r + 3.dp.toPx(), c)
                                drawCircle(accent, r, c)
                            }
                            "miss" -> {
                                for (s in listOf(5.dp.toPx() + 3.dp.toPx() to outline, 5.dp.toPx() to Color(0xFFFF8A80))) {
                                    drawLine(s.second, c + Offset(-r, -r), c + Offset(r, r), strokeWidth = s.first)
                                    drawLine(s.second, c + Offset(-r, r), c + Offset(r, -r), strokeWidth = s.first)
                                }
                            }
                            else -> {
                                drawCircle(outline, r, c, style = Stroke(width = 7.dp.toPx()))
                                drawCircle(Color.White, r, c, style = Stroke(width = 3.dp.toPx()))
                            }
                        }
                    }
                }
            }
        }
        Text("Tap the photo to add a mark", style = MaterialTheme.typography.titleSmall, modifier = Modifier.padding(top = 8.dp))
        FlowRow(Modifier.selectableGroup()) {
            KINDS.forEach { (k, label) ->
                FilterChip(selected = kind == k, onClick = { kind = k }, label = { Text(label) },
                    modifier = Modifier.oneOf().padding(end = 8.dp).heightIn(min = 48.dp).testTag("mark_kind_$k"))
            }
        }
        selected?.takeIf { it in marks.indices }?.let { i ->
            var label by remember(i, marks.size) { mutableStateOf(marks[i].label ?: "") }
            Text("Selected: ${describe(marks[i])}", modifier = Modifier.padding(top = 8.dp))
            Row(verticalAlignment = Alignment.CenterVertically) {
                OutlinedTextField(label, { t -> label = t.take(40); marks[i] = marks[i].copy(label = label.ifBlank { null }) },
                    label = { Text("Label (optional)") }, singleLine = true, modifier = Modifier.weight(1f).testTag("mark_label"))
                TextButton(onClick = { marks.removeAt(i); selected = null }, modifier = Modifier.heightIn(min = 48.dp).testTag("mark_remove")) {
                    Text("Remove")
                }
            }
        }
        val hits = marks.count { it.kind == "hit" }
        val misses = marks.count { it.kind == "miss" }
        Text("$hits hits, $misses misses, ${marks.size - hits - misses} notes", fontWeight = FontWeight.Bold,
            modifier = Modifier.padding(top = 8.dp).testTag("mark_counts"))
        // The marks as text: usable without seeing the photo, each with its own Remove.
        marks.forEachIndexed { i, m ->
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(describe(m), modifier = Modifier.weight(1f))
                TextButton(onClick = { marks.removeAt(i); selected = null }, modifier = Modifier.heightIn(min = 48.dp)
                    .spoken("Remove ${describe(m)}")) { Text("Remove") }
            }
        }
        OutlinedTextField(note, { note = it.take(2_000) }, label = { Text("Note") }, modifier = Modifier.fillMaxWidth().testTag("annotate_note"))
        loaded?.let { a ->
            a.number?.let { n ->
                Text("Version $n, saved with review revision ${a.runRevision ?: "none"}" +
                    (a.scoreRevision?.let { " and score revision $it" } ?: "") + ".", style = MaterialTheme.typography.bodySmall)
            }
            if (a.number != null && (a.currentRunRevision != a.runRevision || a.currentScoreRevision != a.scoreRevision)) {
                Text("The review or score has changed since these marks were saved.", color = MaterialTheme.colorScheme.error,
                    style = MaterialTheme.typography.bodySmall, modifier = Modifier.testTag("annotate_changed"))
            }
        }
        error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
        Button(onClick = {
            scope.launch {
                d.savePhotoAnnotation(photo.id, loaded?.number, marks.toList(), note)
                    .onSuccess { a -> loaded = a; error = null }
                    .onFailure { error = it.message }
            }
        }, modifier = Modifier.fillMaxWidth().heightIn(min = 56.dp).padding(top = 8.dp).testTag("annotate_save")) { Text("Save marks") }
        Text("Marks are your own; Squib does not find hits automatically. They stay on this phone and in backups, and are " +
            "not included when you share a result.", style = MaterialTheme.typography.bodySmall)
    }
}

/** Small photo thumbnails for Review; tapping one opens [PhotoAnnotate]. */
@Composable
fun PhotoThumbs(d: DataController, photos: List<AttachmentView>, onOpen: (AttachmentView) -> Unit) {
    FlowRow {
        photos.forEachIndexed { i, p ->
            var thumb by remember(p.id) { mutableStateOf<ImageBitmap?>(null) }
            LaunchedEffect(p.id) {
                thumb = withContext(Dispatchers.IO) {
                    d.attachmentFile(p.relativePath)?.let { f ->
                        BitmapFactory.decodeFile(f.path, BitmapFactory.Options().apply { inSampleSize = 8 })?.asImageBitmap()
                    }
                }
            }
            TextButton(onClick = { onOpen(p) }, modifier = Modifier.padding(end = 8.dp).testTag("photo_${i}")
                .spoken("Open target photo ${i + 1} to add marks")) {
                thumb?.let { Image(it, contentDescription = null, modifier = Modifier.heightIn(max = 96.dp)) } ?: Text("Photo ${i + 1}")
            }
        }
    }
}

package net.digitalgrease.squib.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import net.digitalgrease.squib.BuildConfig
import net.digitalgrease.squib.core.coreVersions
import org.json.JSONObject

const val SOURCE_URL = "https://github.com/digital-grease/squib"
const val DONATE_URL = "https://www.buymeacoffee.com/digitalgrease"

/** One third-party component and the licence texts that apply to it. */
data class Notice(val name: String, val version: String, val license: String, val ecosystem: String, val texts: List<String>)

private fun loadNotices(json: String): Pair<String, List<Notice>> {
    val o = JSONObject(json)
    val texts = o.getJSONObject("texts")
    val comps = o.getJSONArray("components")
    val list = (0 until comps.length()).map { i ->
        val c = comps.getJSONObject(i)
        val ids = c.getJSONArray("texts")
        Notice(
            c.getString("name"), c.getString("version"), c.getString("license"), c.getString("ecosystem"),
            (0 until ids.length()).map { texts.getString(ids.getString(it)) },
        )
    }
    return o.getString("note") to list
}

/**
 * About (plan 5.5): version, the AGPL notice with the source link and the full licence
 * (offline), a privacy summary matching the README, a Support section, and third-party
 * licence notices generated from the real dependency graphs (scripts/third_party_notices.py).
 */
@Composable
fun AboutScreen(onBack: () -> Unit, onOpenLink: (String) -> Unit) {
    var page by rememberSaveable { mutableStateOf("about") }
    BackHandler(enabled = page != "about") { page = "about" }
    when (page) {
        "licence" -> AssetTextPage("GNU Affero General Public License", "LICENSE.txt", "licence_text") { page = "about" }
        "notices" -> NoticesPage { page = "about" }
        else -> AboutMain(onBack, onOpenLink, onLicence = { page = "licence" }, onNotices = { page = "notices" })
    }
}

@Composable
private fun AboutMain(onBack: () -> Unit, onOpenLink: (String) -> Unit, onLicence: () -> Unit, onNotices: () -> Unit) {
    Column(Modifier.verticalScroll(rememberScrollState()).padding(16.dp).testTag("about")) {
        BackButton("Back", "Back to Data and backup", onBack)
        Text("About Squib", style = MaterialTheme.typography.headlineMedium)
        Text(
            "Version ${BuildConfig.VERSION_NAME} (${BuildConfig.VERSION_CODE})",
            modifier = Modifier.padding(top = 4.dp).testTag("about_version"),
        )
        Text(coreVersions().joinToString(" · "), style = MaterialTheme.typography.bodySmall)

        Section("Free software") {
            Text("Copyright © 2026 the Squib contributors")
            Text(
                "Squib is free software: you can redistribute it and/or modify it under the terms of the GNU Affero " +
                    "General Public License as published by the Free Software Foundation, either version 3 of the " +
                    "License, or (at your option) any later version. It is distributed in the hope that it will be " +
                    "useful, but WITHOUT ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS " +
                    "FOR A PARTICULAR PURPOSE. See the licence for details.",
                style = MaterialTheme.typography.bodySmall,
                modifier = Modifier.padding(top = 4.dp),
            )
            OutlinedButton(onClick = onLicence, modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp).padding(top = 6.dp).testTag("about_licence")) {
                Text("Read the licence")
            }
            OutlinedButton(onClick = { onOpenLink(SOURCE_URL) }, modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp).testTag("about_source")) {
                Text("Source code (opens github.com)")
            }
            OutlinedButton(onClick = onNotices, modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp).testTag("about_notices")) {
                Text("Third-party licences")
            }
        }

        Section("Privacy") {
            for (line in PRIVACY) Text("• $line", style = MaterialTheme.typography.bodySmall, modifier = Modifier.padding(top = 2.dp))
        }

        Section("Support") {
            Text(
                "Squib is free, with no ads, no in-app purchases and no tracking. If it helps your practice and you " +
                    "would like to support its development, you can buy the developer a coffee. It is optional and " +
                    "changes nothing in the app.",
                style = MaterialTheme.typography.bodySmall,
            )
            Button(onClick = { onOpenLink(DONATE_URL) }, modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp).padding(top = 6.dp).testTag("about_donate")) {
                Text("Buy me a coffee (opens buymeacoffee.com)")
            }
        }
    }
}

/** Kept in step with the README's Privacy section. */
private val PRIVACY = listOf(
    "No account, no ads, and no tracking.",
    "The internet is used only for two lookups, each off until you turn it on: weather (your place rounded to about 1 km, " +
        "sent to the US National Weather Service or the NOAA Aviation Weather Center) and US terrain elevation (your place " +
        "to about 10 m, sent to the US Geological Survey).",
    "Audio is never sent anywhere and, by default, never saved: sound is analysed in memory and discarded. A diagnostic " +
        "recording keeps one run's audio only when you switch it on for that run.",
    "The microphone, camera and location are requested only when you use a feature that needs them.",
    "History stays on this phone and is not in Android's automatic backups. Backup files are encrypted only when you choose " +
        "a passphrase while saving; either way, store them somewhere you trust.",
    "Shared results leave out your location, station names, notes, photos, videos and identifiers.",
    "Problem and compatibility reports open in your browser for you to review and submit; nothing is sent by itself.",
    "Links on this screen open in your browser and leave the app.",
)

@Composable
private fun Section(title: String, content: @Composable () -> Unit) {
    Card(Modifier.fillMaxWidth().padding(vertical = 6.dp)) {
        Column(Modifier.padding(12.dp)) {
            Text(title, fontWeight = FontWeight.Bold)
            content()
        }
    }
}

@Composable
private fun AssetTextPage(title: String, asset: String, tag: String, onBack: () -> Unit) {
    val context = LocalContext.current
    var text by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(asset) {
        text = withContext(Dispatchers.IO) { runCatching { context.assets.open(asset).bufferedReader().readText() }.getOrNull() }
    }
    // Paragraphs as separate items: the licence is long, and one Text would lay it all out at once.
    val paragraphs = remember(text) { text?.split(Regex("\n\\s*\n"))?.map { it.trim() }?.filter { it.isNotEmpty() } ?: emptyList() }
    LazyColumn(Modifier.padding(horizontal = 16.dp).testTag(tag)) {
        item {
            BackButton("Back", "Back to About", onBack)
            Text(title, style = MaterialTheme.typography.headlineSmall, modifier = Modifier.padding(bottom = 8.dp))
            if (text == null) Text("Loading…")
        }
        items(paragraphs) { p -> Text(p, style = MaterialTheme.typography.bodySmall, modifier = Modifier.padding(bottom = 8.dp)) }
    }
}

@Composable
private fun NoticesPage(onBack: () -> Unit) {
    val context = LocalContext.current
    var data by remember { mutableStateOf<Pair<String, List<Notice>>?>(null) }
    var open by rememberSaveable { mutableStateOf<String?>(null) }
    LaunchedEffect(Unit) {
        data = withContext(Dispatchers.IO) {
            runCatching { loadNotices(context.assets.open("third_party_notices.json").bufferedReader().readText()) }.getOrNull()
        }
    }
    LazyColumn(Modifier.padding(horizontal = 16.dp).testTag("notices_list")) {
        item {
            BackButton("Back", "Back to About", onBack)
            Text("Third-party licences", style = MaterialTheme.typography.headlineSmall)
            val d = data
            if (d == null) {
                Text("Loading…")
            } else {
                val rust = d.second.count { it.ecosystem == "rust" }
                Text(
                    "Squib is built on $rust Rust crates and ${d.second.size - rust} Android libraries, listed with their " +
                        "licences. Tap one to read its licence text. ${d.first}",
                    style = MaterialTheme.typography.bodySmall,
                    modifier = Modifier.padding(vertical = 8.dp),
                )
            }
        }
        items(data?.second ?: emptyList(), key = { "${it.ecosystem}:${it.name}:${it.version}" }) { n ->
            val key = "${n.ecosystem}:${n.name}:${n.version}"
            val expanded = open == key
            Column(
                Modifier.fillMaxWidth().heightIn(min = 48.dp)
                    .semantics { stateDescription = if (expanded) "Expanded" else "Collapsed" }
                    .clickable(role = Role.Button) { open = if (expanded) null else key }
                    .padding(vertical = 6.dp),
            ) {
                Text("${n.name} ${n.version}", fontWeight = FontWeight.Bold)
                Text(n.license, style = MaterialTheme.typography.bodySmall)
                if (expanded) {
                    for (t in n.texts) Text(t, style = MaterialTheme.typography.bodySmall, modifier = Modifier.padding(top = 6.dp))
                }
            }
        }
    }
}

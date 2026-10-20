package dev.digitalgrease.squib

import android.app.Activity
import android.app.Instrumentation
import android.content.Intent
import android.net.Uri
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performScrollToNode
import androidx.compose.ui.test.performTextClearance
import androidx.compose.ui.test.performTextInput
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Backup and restore through the Data screen (A17; plan 5.7 encryption) on an emulator.
 * The system file picker is replaced by a blocking monitor that answers with a file in
 * the app's own storage, so the app's real save, open, decrypt, preview and import code
 * runs end to end.
 */
@RunWith(AndroidJUnit4::class)
class BackupFlowTest {
    @get:Rule
    val rule = createAndroidComposeRule<MainActivity>()

    private fun tag(t: String) = rule.onAllNodes(hasTestTag(t)).fetchSemanticsNodes().isNotEmpty()
    private fun has(t: String) = rule.onAllNodes(hasText(t, substring = true)).fetchSemanticsNodes().isNotEmpty()

    /** Tap, answer the picker intent with `file`, and return the intent the app sent. */
    private fun tapAndPick(tag: String, action: String, file: File): Intent {
        var seen: Intent? = null
        val monitor = object : Instrumentation.ActivityMonitor() {
            override fun onStartActivity(intent: Intent): Instrumentation.ActivityResult? {
                if (intent.action != action) return null
                seen = intent
                return Instrumentation.ActivityResult(Activity.RESULT_OK, Intent().setData(Uri.fromFile(file)))
            }
        }
        val inst = InstrumentationRegistry.getInstrumentation()
        inst.addMonitor(monitor)
        try {
            rule.onNodeWithTag(tag).performScrollTo().performClick()
            rule.waitUntil(5_000) { seen != null }
            return seen!!
        } finally {
            inst.removeMonitor(monitor)
        }
    }

    private fun dismissMessage(expect: String) {
        rule.waitOrDump(30_000) { has(expect) }
        rule.onNodeWithText("OK").performClick()
        rule.waitForIdle()
    }

    private fun type(tag: String, text: String, scroll: Boolean = true) {
        if (scroll) rule.onNodeWithTag(tag).performScrollTo()
        rule.onNodeWithTag(tag).performTextClearance()
        rule.onNodeWithTag(tag).performTextInput(text)
    }

    @Test
    fun encryptedAndPlainBackupsSaveAndRestore() {
        // No app internals here, so this also runs on the minified release build.
        val app = rule.activity.application
        val dir = File(app.filesDir, "backup-test").apply { deleteRecursively(); mkdirs() }
        val enc = File(dir, "picked.zip.age")
        val plain = File(dir, "picked.zip")

        rule.onNodeWithTag("tab_history").performClick()
        rule.waitOrDump { tag("history") }
        rule.onNodeWithTag("history").performScrollToNode(hasTestTag("open_data"))
        rule.onNodeWithTag("open_data").performClick()
        rule.waitOrDump { tag("backup_encrypt") }
        check(has("Unless you encrypt it with a passphrase")) { "the export says when a backup is unencrypted" }

        // Encryption needs a long enough passphrase, typed the same twice.
        rule.onNodeWithTag("backup_encrypt").performScrollTo().performClick()
        rule.waitOrDump { tag("backup_pass1") }
        check(has("there is no recovery")) { "loss warning" }
        rule.onNodeWithTag("backup_save").assertIsNotEnabled()
        type("backup_pass1", "short")
        rule.waitOrDump { has("at least 8 characters") }
        rule.onNodeWithTag("backup_save").assertIsNotEnabled()
        type("backup_pass1", "correct horse battery")
        type("backup_pass2", "correct horse batter")
        rule.waitOrDump { has("do not match") }
        rule.onNodeWithTag("backup_save").assertIsNotEnabled()
        type("backup_pass2", "correct horse battery")
        rule.waitOrDump { !tag("backup_pass_problem") }
        rule.onNodeWithTag("backup_save").assertIsEnabled()

        val create = tapAndPick("backup_save", Intent.ACTION_CREATE_DOCUMENT, enc)
        check(create.getStringExtra(Intent.EXTRA_TITLE) == "squib-backup.zip.age") { "suggested name ${create.extras}" }
        check(create.type == "application/octet-stream") { "generic type keeps the .age name: ${create.type}" }
        dismissMessage("Encrypted backup saved")
        val head = enc.readBytes()
        check(String(head, 0, 22, Charsets.US_ASCII) == "age-encryption.org/v1\n") { "an age file" }
        check(!String(head, Charsets.ISO_8859_1).contains("manifest.json")) { "no plaintext zip structure" }
        check(app.cacheDir.listFiles().orEmpty().none { it.name.startsWith("squib-plain") || it.name.startsWith("squib-backup") }) {
            "no plaintext or staging copy left in cache: ${app.cacheDir.list()?.toList()}"
        }

        // Restore asks for the passphrase; a wrong one is refused and can be retried.
        tapAndPick("backup_restore", Intent.ACTION_OPEN_DOCUMENT, enc)
        rule.waitOrDump { tag("import_passphrase") }
        type("import_passphrase", "wrong horse battery", scroll = false)
        rule.onNodeWithTag("import_unlock").performClick()
        rule.waitOrDump(20_000) { has("Wrong passphrase.") }
        type("import_passphrase", "correct horse battery", scroll = false)
        rule.onNodeWithTag("import_unlock").performClick()
        rule.waitOrDump(20_000) { tag("import_restore") }
        check(has("Backup checked.")) { "preview after decrypting" }
        rule.onNodeWithTag("import_restore").performClick()
        dismissMessage("Imported 0 new runs")
        check(app.cacheDir.listFiles().orEmpty().none { it.name.startsWith("squib-plain") || it.name == "squib-import.bin" }) {
            "decrypted and picked copies removed: ${app.cacheDir.list()?.toList()}"
        }

        // Unencrypted stays available and restores without a passphrase.
        rule.onNodeWithTag("backup_encrypt").performScrollTo().performClick()
        rule.waitOrDump { !tag("backup_pass1") }
        val createPlain = tapAndPick("backup_save", Intent.ACTION_CREATE_DOCUMENT, plain)
        check(createPlain.getStringExtra(Intent.EXTRA_TITLE) == "squib-backup.zip") { "plain name" }
        dismissMessage("It is not encrypted")
        val zip = plain.readBytes()
        check(zip.take(2) == listOf('P'.code.toByte(), 'K'.code.toByte()) && String(zip, Charsets.ISO_8859_1).contains("manifest.json")) { "a zip" }
        tapAndPick("backup_restore", Intent.ACTION_OPEN_DOCUMENT, plain)
        rule.waitOrDump(20_000) { tag("import_restore") }
        check(!tag("import_passphrase"))
        rule.onNodeWithTag("import_restore").performClick()
        dismissMessage("Imported 0 new runs")
        dir.deleteRecursively()
    }
}

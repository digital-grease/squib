package net.digitalgrease.squib

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.media.ExifInterface
import android.net.Uri
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import net.digitalgrease.squib.data.DataController
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/** Plan phase 1.6 on an emulator: imported photos are upright and carry no EXIF/GPS. */
@RunWith(AndroidJUnit4::class)
class PhotoImportTest {
    @Test
    fun rotatedPhotoIsStoredUprightWithoutMetadata() {
        val inst = InstrumentationRegistry.getInstrumentation()
        val app = inst.targetContext.applicationContext as SquibApp
        // A landscape 200x100 JPEG whose EXIF says "rotate 90 degrees to display", with GPS.
        val src = File(app.cacheDir, "rotated-test.jpg")
        val bmp = Bitmap.createBitmap(200, 100, Bitmap.Config.ARGB_8888)
        src.outputStream().use { bmp.compress(Bitmap.CompressFormat.JPEG, 90, it) }
        ExifInterface(src.path).apply {
            setAttribute(ExifInterface.TAG_ORIENTATION, ExifInterface.ORIENTATION_ROTATE_90.toString())
            setAttribute(ExifInterface.TAG_GPS_LATITUDE, "40/1,0/1,0/1")
            setAttribute(ExifInterface.TAG_GPS_LATITUDE_REF, "N")
            setAttribute(ExifInterface.TAG_GPS_LONGITUDE, "105/1,0/1,0/1")
            setAttribute(ExifInterface.TAG_GPS_LONGITUDE_REF, "W")
            saveAttributes()
        }
        val runId = app.engine.addManualRun("Club timer", 10u, "1.5", null, null, System.currentTimeMillis(), 0, "test")

        val done = CountDownLatch(1)
        lateinit var data: DataController
        inst.runOnMainSync {
            data = DataController(app)
            data.addPhoto(runId, Uri.fromFile(src)) { done.countDown() }
        }
        assertTrue("photo import finished", done.await(20, TimeUnit.SECONDS))

        val stored = app.engine.runAttachments(runId).single()
        val file = File(data.attachmentRoot, stored.relativePath)
        val b = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeFile(file.path, b)
        assertEquals("rotated into the pixels", 100, b.outWidth)
        assertEquals(200, b.outHeight)
        val exif = ExifInterface(file.path)
        val o = exif.getAttributeInt(ExifInterface.TAG_ORIENTATION, ExifInterface.ORIENTATION_UNDEFINED)
        assertTrue("no rotation left for viewers to apply twice: $o", o == ExifInterface.ORIENTATION_UNDEFINED || o == ExifInterface.ORIENTATION_NORMAL)
        assertTrue("the source really had GPS", ExifInterface(src.path).getLatLong(FloatArray(2)))
        assertNull("no GPS", exif.getAttribute(ExifInterface.TAG_GPS_LATITUDE))
        assertTrue(!exif.getLatLong(FloatArray(2)))
        src.delete()
    }
}

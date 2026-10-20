package dev.digitalgrease.squib.data

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class PhotoPrepTest {
    @Test
    fun exifOrientationsMapToRotationAndMirror() {
        assertTrue(PhotoPrep.transform(1).isIdentity)
        assertTrue("unknown values leave the image alone", PhotoPrep.transform(0).isIdentity)
        assertTrue(PhotoPrep.transform(99).isIdentity)
        assertEquals(PhotoPrep.Transform(90, false), PhotoPrep.transform(6))
        assertEquals(PhotoPrep.Transform(270, false), PhotoPrep.transform(8))
        assertEquals(PhotoPrep.Transform(180, false), PhotoPrep.transform(3))
        assertEquals(PhotoPrep.Transform(0, true), PhotoPrep.transform(2))
        assertEquals(PhotoPrep.Transform(180, true), PhotoPrep.transform(4))
        assertEquals(PhotoPrep.Transform(270, true), PhotoPrep.transform(5))
        assertEquals(PhotoPrep.Transform(90, true), PhotoPrep.transform(7))
    }

    @Test
    fun sampleSizeBoundsTheLongestEdge() {
        assertEquals(1, PhotoPrep.sampleSize(4000, 3000))
        assertEquals(2, PhotoPrep.sampleSize(8160, 6120))
        assertEquals(4, PhotoPrep.sampleSize(3000, 16384))
        assertEquals(1, PhotoPrep.sampleSize(0, 0))
        assertTrue(maxOf(12000, 9000) / PhotoPrep.sampleSize(12000, 9000) <= PhotoPrep.MAX_EDGE_PX)
    }
}

package net.digitalgrease.squib.data

/**
 * Pure helpers for importing target photos: the transform an EXIF orientation implies,
 * and a power-of-two decode sample size that bounds memory. Kept free of Android types
 * so they can be unit tested on the JVM.
 */
object PhotoPrep {
    /** Longest edge kept after import; larger photos are decoded smaller. */
    const val MAX_EDGE_PX = 4096

    /** Rotation in degrees (clockwise) and whether to mirror horizontally first. */
    data class Transform(val rotateDeg: Int, val mirror: Boolean) {
        val isIdentity get() = rotateDeg == 0 && !mirror
    }

    /**
     * EXIF orientation values 1 to 8 (TIFF/EXIF spec): 1 normal, 2 mirror, 3 rotate 180,
     * 4 mirror + 180, 5 mirror + 270, 6 rotate 90, 7 mirror + 90, 8 rotate 270.
     * Unknown or undefined values leave the image as stored.
     */
    fun transform(exifOrientation: Int): Transform = when (exifOrientation) {
        2 -> Transform(0, true)
        3 -> Transform(180, false)
        4 -> Transform(180, true)
        5 -> Transform(270, true)
        6 -> Transform(90, false)
        7 -> Transform(90, true)
        8 -> Transform(270, false)
        else -> Transform(0, false)
    }

    /** Smallest power-of-two sample size that brings the longest edge within `maxEdge`. */
    fun sampleSize(width: Int, height: Int, maxEdge: Int = MAX_EDGE_PX): Int {
        if (width <= 0 || height <= 0) return 1
        var s = 1
        while (maxOf(width, height) / s > maxEdge) s *= 2
        return s
    }
}

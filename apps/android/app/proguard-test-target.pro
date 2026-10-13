# Added to the release rules only for -PsquibTestBuildType=release. The instrumented tests
# run in the app's process and share its copy of these libraries, which R8 trims to what the
# app itself uses. Keeping them whole lets the tests load; Squib's own code is still shrunk
# and obfuscated, and the JNA, UniFFI and JNI rules apply exactly as in a shipping build.
# Shipped builds never include this file.
-keep class kotlin.** { *; }
-keep class kotlinx.** { *; }
-keep class androidx.** { *; }

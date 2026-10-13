# Applies only to the instrumented-test APK when tests run against the minified release
# (-PsquibTestBuildType=release). These are compile-time annotations referenced by test
# libraries (the accessibility test framework), absent at run time.
-dontwarn com.google.auto.value.**
-dontwarn javax.lang.model.**
-dontwarn com.google.errorprone.annotations.**

# The test classes themselves are found by name and annotation: keep them unchanged. Only
# the app under test is meant to be minified.
-keep @org.junit.runner.RunWith class * { *; }
-keep class net.digitalgrease.squib.Soak
-keepattributes *Annotation*,Signature,InnerClasses,EnclosingMethod
-keep class org.junit.** { *; }
-keep class androidx.test.** { *; }

# Release shrinking rules (plan 5.4, ADR-058). Every rule here protects code reached by
# name or by reflection from native code, which R8 cannot see.

# JNA: loads its own native dispatch library and reads classes, fields and methods by name.
-keep class com.sun.jna.** { *; }
-keep class * implements com.sun.jna.** { *; }
-dontwarn java.awt.**

# UniFFI bindings (generated): JNA direct-maps the scaffolding's external functions by name
# and reads Structure fields (RustBuffer, call status) by reflection. Kept whole: the package
# is small and every part of it crosses the native boundary.
-keep class dev.digitalgrease.squib.core.** { *; }

# JNI data plane: Java_dev_digitalgrease_squib_audio_PcmBridge_nativePush is looked up by
# the class and method name.
-keepclasseswithmembernames,includedescriptorclasses class * {
    native <methods>;
}
-keep class dev.digitalgrease.squib.audio.PcmBridge { *; }

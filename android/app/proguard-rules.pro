# Keep UniFFI-generated classes (Ketra.kt) — they're referenced via reflection
# by the JNI bridge and shouldn't be renamed or removed.
-keep class io.ketracopy.uniffi.** { *; }
-keep class uniffi.** { *; }

# OkHttp (used by the WebDAV provider).
-dontwarn okhttp3.**
-dontwarn okio.**

package app.outl.mobile_app

import android.content.Context
import android.os.Bundle
import androidx.activity.enableEdgeToEdge

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    // Prime the JVM-backed globals iroh's TLS verifier + system-DNS reader
    // need on Android, BEFORE super.onCreate() boots Tauri (and the iroh
    // transport). Without this the first QUIC/TLS connection panics
    // (uninitialized rustls-platform-verifier / ndk_context) and the process
    // aborts with SIGABRT on a tokio worker thread. See android_jni.rs.
    NativeSetup.install(applicationContext)
    super.onCreate(savedInstanceState)
  }
}

/// Loads the Rust lib and bridges to `Java_app_outl_mobile_1app_NativeSetup_install`.
/// Separate from Tauri's generated `Rust` object so the JNI symbol name is stable.
private object NativeSetup {
  init { System.loadLibrary("outl_mobile_lib") }

  @JvmStatic external fun install(context: Context)
}

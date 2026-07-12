//! Android JNI bootstrap for iroh's TLS + DNS.
//!
//! iroh's relay client verifies certificates through `rustls-platform-verifier`
//! and reads the system DNS config through `iroh-dns` / `hickory` — both call
//! into the JVM (the Android KeyStore and `ConnectivityManager`). Neither
//! Tauri/wry nor `tao`'s bundled `ndk_glue` installs the process-wide JNI
//! context those crates read: the verifier keeps its own `OnceCell`, and the
//! DNS reader goes through `ndk_context`, and both start out empty.
//!
//! So on the first QUIC connection the JNI lookups panic; the panic then
//! poisons a `quinn` (`noq`) mutex, and the next task's `.unwrap()` on the
//! `PoisonError` aborts the process — surfacing as a `SIGABRT` on a
//! `tokio-rt-worker` thread moments after boot.
//!
//! `MainActivity.onCreate` calls [`Java_app_outl_mobile_1app_NativeSetup_install`]
//! **before** `super.onCreate` boots Tauri (and therefore before iroh makes its
//! first connection), handing us the `Application` context so we can prime both
//! globals exactly once, up front.

use std::ffi::c_void;

use jni::objects::{JClass, JObject};
use jni::{Env, EnvUnowned};

/// JNI entry point invoked from `MainActivity` (`NativeSetup.install`).
///
/// Primes `rustls-platform-verifier`'s global JNI handle and `ndk_context` so
/// iroh's TLS verification and system-DNS reads work on Android. `with_env`
/// wraps the body in `catch_unwind`, and [`jni::errors::LogErrorAndDefault`]
/// logs (never throws) on failure — a thrown exception here would crash
/// `onCreate`. If priming fails the app still boots; iroh may then misbehave,
/// but that is recoverable where an aborted process is not.
///
/// # Safety
///
/// Called by the JVM with a valid JNI env, class, and non-null `Context`.
#[no_mangle]
pub extern "system" fn Java_app_outl_mobile_1app_NativeSetup_install<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    context: JObject<'local>,
) {
    unowned_env
        .with_env(|env: &mut Env| install(env, context))
        .resolve::<jni::errors::LogErrorAndDefault>();
}

/// Install both JVM-backed globals iroh needs. Runs once at boot.
fn install(env: &mut Env, context: JObject) -> Result<(), jni::errors::Error> {
    // A stable reference to the `Application` context for `ndk_context`, which
    // stores raw pointers and expects the referent to outlive the process. The
    // global ref is intentionally leaked below (`mem::forget`) so it is never
    // released.
    let ctx_global = env.new_global_ref(&context)?;
    let vm = env.get_java_vm()?;

    // TLS cert verification (iroh relay client → rustls). The verifier reads
    // the Android system trust store via JNI and `.expect(...)`s its `OnceCell`
    // on the first handshake — prime it now. Consumes the local `context`.
    rustls_platform_verifier::android::init_with_env(env, context)?;

    // System DNS (iroh-dns → hickory → `LinkProperties.getDnsServers()` via
    // JNI). Reads the JavaVM + Context straight out of `ndk_context`.
    //
    // SAFETY: `vm.get_raw()` is valid for the process lifetime, and the leaked
    // global ref keeps its jobject alive just as long, satisfying
    // `initialize_android_context`'s contract.
    unsafe {
        ndk_context::initialize_android_context(
            vm.get_raw() as *mut c_void,
            ctx_global.as_obj().as_raw() as *mut c_void,
        );
    }
    std::mem::forget(ctx_global);

    tracing::info!("android JNI context installed (rustls-platform-verifier + ndk_context)");
    Ok(())
}

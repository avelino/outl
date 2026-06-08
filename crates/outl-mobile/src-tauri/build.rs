fn main() {
    tauri_build::build();

    // iOS workaround for `mlua-sys` vendored libLua.
    //
    // `mlua-sys` emits `cargo:rustc-link-lib=static:-bundle=lua5.4`
    // when its `vendored` feature builds the C interpreter. The
    // `-bundle` modifier keeps `liblua5.4.a`'s object files out of
    // our crate's archive — every desktop target is fine because the
    // downstream linker (rustc / cc / lld) re-runs and pulls the
    // sibling archive in via the `-L` search path.
    //
    // Tauri iOS doesn't re-run a Rust linker on the final binary: it
    // hands the staticlib (`libapp.a`) straight to Xcode, which only
    // sees the one archive. Without the Lua objects baked in, every
    // `_lua_*` / `_luaopen_*` symbol comes back unresolved at the
    // `Ld outl.app/outl.debug.dylib` stage.
    //
    // Re-emit the lib with `+bundle:+whole-archive` so its objects
    // land inside `libapp.a`. Cargo's rule is "last directive wins"
    // for the same link-lib name, so this overrides the `-bundle`
    // emitted by `mlua-sys`. Side-effect: the iOS IPA grows by
    // roughly libLua's ~200 KB of compiled code. Acceptable cost for
    // having `lang-lua` shipped on mobile.
    //
    // Gated strictly on `target_os = "ios"` so desktop / TUI / CLI
    // builds keep the upstream `-bundle` behaviour and don't pay the
    // bundle cost.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("ios") {
        println!("cargo:rustc-link-lib=static:+bundle,+whole-archive=lua5.4");
    }
}

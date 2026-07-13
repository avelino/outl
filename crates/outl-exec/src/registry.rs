//! Map from fence info-string (`"lisp"`, `"python"`, ...) to runtime.
//!
//! Built-in runtimes are registered via [`RuntimeRegistry::default`].
//! Hosts can layer more at startup with [`RuntimeRegistry::register`],
//! or discover drop-in `.wasm` modules with
//! `RuntimeRegistry::discover_wasm_dir` (M2 — see TODO inside).

use std::collections::HashMap;
use std::sync::Arc;

use crate::runtime::Runtime;
use crate::runtimes;

/// Owned set of runtimes, keyed by the lowercased fence info-string.
#[derive(Clone, Default)]
pub struct RuntimeRegistry {
    by_lang: HashMap<String, Arc<dyn Runtime>>,
}

impl RuntimeRegistry {
    /// Empty registry. Most callers want [`RuntimeRegistry::with_builtins`].
    pub fn new() -> Self {
        Self {
            by_lang: HashMap::new(),
        }
    }

    /// New registry pre-populated with every shipped runtime.
    ///
    /// Each language ships behind a feature (`lang-lisp`, `lang-js`,
    /// `lang-python`, `lang-lua`) so binaries can strip what they don't
    /// need. `echo` is the smoke-test runtime — gated behind
    /// `debug_assertions` / `test` so release builds (notably the iOS
    /// IPA) don't surface an "echo" language to the App Store
    /// reviewer; it has no production value and reads as a dev hook.
    pub fn with_builtins() -> Self {
        let mut r = Self::new();
        #[cfg(any(test, debug_assertions))]
        r.register(runtimes::echo::EchoRuntime);
        #[cfg(feature = "lang-lisp")]
        r.register(runtimes::lisp::LispRuntime);
        #[cfg(feature = "lang-js")]
        r.register(runtimes::js::JsRuntime);
        #[cfg(feature = "lang-python")]
        r.register(runtimes::python::PythonRuntime);
        #[cfg(feature = "lang-lua")]
        r.register(runtimes::lua::LuaRuntime);
        #[cfg(feature = "lang-rust")]
        r.register(runtimes::rust::RustRuntime::default());
        #[cfg(feature = "lang-query")]
        r.register(runtimes::query::QueryRuntime);
        r
    }

    /// Insert (or replace) a runtime. The lookup key is the runtime's
    /// own `language()`, lowercased.
    pub fn register<R: Runtime + 'static>(&mut self, r: R) -> &mut Self {
        self.by_lang
            .insert(r.language().to_ascii_lowercase(), Arc::new(r));
        self
    }

    /// Look up a runtime by fence info-string. Returns `None` if no
    /// runtime is registered for that language.
    ///
    /// The input is run through [`outl_md::lang::canonical`] first
    /// so user-written aliases (`javascript`, `node`, `rs`, `py3`,
    /// …) resolve to the registry key the runtime registered with
    /// (`js`, `rust`, `python`). The original lower-cased form is
    /// the fallback when no alias matches — keeps the door open for
    /// runtimes registered out-of-band that don't have an alias
    /// entry yet.
    pub fn get(&self, lang: &str) -> Option<Arc<dyn Runtime>> {
        let key = outl_md::lang::canonical(lang)
            .map(str::to_owned)
            .unwrap_or_else(|| lang.to_ascii_lowercase());
        self.by_lang.get(&key).cloned()
    }

    /// Every registered language. Useful for `:run ?` style help.
    pub fn languages(&self) -> impl Iterator<Item = &str> {
        self.by_lang.keys().map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_registers_echo() {
        let r = RuntimeRegistry::with_builtins();
        assert!(r.get("echo").is_some());
    }

    #[cfg(feature = "lang-lisp")]
    #[test]
    fn registers_lisp_when_feature_on() {
        let r = RuntimeRegistry::with_builtins();
        assert!(r.get("lisp").is_some());
    }

    #[test]
    fn lookup_is_case_insensitive() {
        let r = RuntimeRegistry::with_builtins();
        assert!(r.get("ECHO").is_some());
        assert!(r.get("Echo").is_some());
    }

    #[test]
    fn unknown_language_returns_none() {
        let r = RuntimeRegistry::with_builtins();
        assert!(r.get("klingon").is_none());
    }
}

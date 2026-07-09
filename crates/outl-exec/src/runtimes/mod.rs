//! Built-in runtimes — one file per language, each behind a feature.
//!
//! Adding a new language is the smallest possible change:
//!
//! 1. Pick a mature embeddable Rust crate for the interpreter.
//! 2. Drop it under `cargo add` behind an optional `dep:` and add a
//!    `lang-<name>` feature in `Cargo.toml`.
//! 3. Create `runtimes/<name>.rs` with one struct + one `impl Runtime`.
//!    Use existing files as a template — most are ~80 lines.
//! 4. Register it in `RuntimeRegistry::with_builtins`.
//!
//! That's it. No parser, no tokenizer, no eval written by hand — the
//! interpreter crate owns the language; we own the plumbing.

pub mod echo;

#[cfg(feature = "lang-lisp")]
pub mod lisp;

#[cfg(feature = "lang-js")]
pub mod js;

#[cfg(feature = "lang-python")]
pub mod python;

#[cfg(feature = "lang-lua")]
pub mod lua;

#[cfg(feature = "lang-rust")]
pub mod rust;

#[cfg(feature = "lang-query")]
pub mod query;

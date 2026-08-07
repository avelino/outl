# RFC 0025 — iOS bans JIT, so the plugin runtime is an interpreter

| | |
|---|---|
| **Status** | Shipped |
| **Issue** | [#25](https://github.com/avelino/outl/issues/25) (supporting: [#4](https://github.com/avelino/outl/issues/4)) |
| **PR** | — |
| **Date** | 2026-08-06 |
| **Reference doc** | [plugins.md](../plugins.md), [plugin-architecture.md](../plugin-architecture.md) |
| **Invariant** | none |
| **Guarded by** | `gas_interrupts_runaway_recursion`, `script_error_surfaces` (`crates/outl-plugins/src/engine.rs`) — these pin the interpreter's cooperative gas, **not** the engine choice; no test can assert "this binary embeds no JIT" in-process, so for the decision itself: **none found — gap** |

## Why

The plugin system is the most documented feature in the repo.
Four docs already cover the manifest, capabilities, permissions, packaging and the host API:
[`plugins.md`](../plugins.md), [`plugin-api.md`](../plugin-api.md), [`plugin-architecture.md`](../plugin-architecture.md), [`plugin-tutorial.md`](../plugin-tutorial.md).
[#25](https://github.com/avelino/outl/issues/25) carries the full approved design in its own body.

Exactly one decision shaped every other one and is written nowhere as a *reason* a contributor reads: **which JavaScript engine**.
It looks like a performance question and is not.
Getting it wrong does not make plugins slow on mobile — it makes them impossible to ship, because outl targets iOS and a plugin is user-installed code.

## What we chose

**Boa**, a pure-Rust JavaScript interpreter, behind the `PluginEngine` trait.

iOS forbids JIT compilation of downloaded code, which eliminates V8, `deno_core` and JavaScriptCore-with-JIT before any benchmark runs.
The survivors are interpreters; AOT is not available for code the user installs after shipping.

Boa was additionally **already embedded in `outl-exec`** for ` ```js ` code blocks, so choosing it added no engine, no C dependency, and one context setup to share instead of two to maintain.
`PluginEngine` keeps the door open: moving to QuickJS is allowed **only if** gas, perf or async becomes a *measured* blocker, never an anticipated one.

Everything else about the system is owned elsewhere and deliberately not restated here.
The runtime rule lives at `crates/outl-plugins/CLAUDE.md`, the architecture at [`plugin-architecture.md`](../plugin-architecture.md), the contract at [`plugin-api.md`](../plugin-api.md).

## Why not the alternatives

**V8 or `deno_core`.**
Fastest, largest ecosystem, and JIT — banned on iOS.
Also tens of megabytes against a project that ships one small binary per platform.

**JavaScriptCore.**
Present on iOS, but third-party in-process embedders get it JIT-disabled anyway, and it is an Apple framework rather than something portable to Linux and Windows on the same terms.

**QuickJS via `rquickjs`** — which is what #25's body actually proposed, with Boa as the fallback.
The shipped choice inverted that, and this is the record of why: QuickJS is a C dependency and would have been a *second* JS engine in a tree that already had Boa running code blocks.
It remains the documented escape hatch behind `PluginEngine`.

**WASM via `wasmtime` / `extism`.**
Better isolation, and a JIT-free interpreter mode exists.
It costs the goal — plugin authors would write Rust or Go, losing the "largest community, lowest barrier" premise of #25, and [#4](https://github.com/avelino/outl/issues/4) had already called WASM isolation heavier than day-one needs.

**Rhai or a bespoke scripting language.**
No ecosystem, nothing an author already knows, every helper written from scratch.

## The opposite direction

**What this makes worse: no JIT means no speed, and the cost lands on the clients.**
Boa's `Context` is single-threaded, so `PluginHost` is `!Send` — the TUI and CLI drive it inline while both GUI clients must run it on a dedicated thread behind an `mpsc` hop.
One host, two plumbings, permanently.

Protection against a misbehaving plugin is **cooperative gas**, not a wall-clock timeout: caps on loop iterations (~20M), recursion (~2000) and stack depth.
A plugin that is merely slow but terminating still blocks the turn it runs on, and the user sees a stall with no error.

**The mirrored case, stated explicitly.**
If a measured blocker ever justifies QuickJS, iOS still forbids its JIT mode, so the platform that motivated this constraint gains nothing — only desktop and CLI get faster.
Worth naming because it means "switch engines for perf" can never be a *portability* fix, and any future proposal framed that way is answering the wrong question.

## How it cannot regress

1. **The rule.**
   `crates/outl-plugins/CLAUDE.md` states it at the top together with the escape condition ("only if gas/perf/async becomes a *measured* blocker").
   [`plugin-architecture.md`](../plugin-architecture.md) restates it where the diagram is, so a reader tracing the runtime hits the same reason.
2. **The tests.**
   `gas_interrupts_runaway_recursion` and `script_error_surfaces` pin the limits an interpreter makes possible.
   The engine choice itself has no mechanical guard, and that is a real gap, not an omission.

## Scope

**Not covered — anything else about the plugin system.**
Manifest, capabilities, permission model, lockfile, `.outlpkg`, dev mode and the registry belong to the four plugin docs above and the status list in `crates/outl-plugins/CLAUDE.md`.

**Not covered — the remaining wiring** (`.outlpkg` packaging, `github:` install source, `network` and `storage` host calls), tracked in that crate's status section.

**Not covered — performance budgets, cold-start cost, memory caps.**
#25 declined to guess at these, and they still need measurement on real plugins.

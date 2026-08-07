# RFC 0139 — A line-oriented query DSL in a code fence, not datalog

| | |
|---|---|
| **Status** | Shipped |
| **Issue** | [#139](https://github.com/avelino/outl/issues/139) |
| **PR** | — |
| **Date** | 2026-08-06 |
| **Reference doc** | [query.md](../query.md) |
| **Invariant** | none |
| **Guarded by** | `parses_status_todo`, `parses_multiple_filters`, `ignores_comments`, `parses_sort`, `parses_since`, `rejects_unknown_key` (the `dsl` tests in `crates/outl-exec/src/runtimes/query.rs`), `split_todo_open`, `split_todo_done`, `split_todo_none` (the `engine` tests in the same file) |

## Why

[#139](https://github.com/avelino/outl/issues/139) states the pain plainly: tasks are scattered across many notes and get lost, and the workaround is searching for them by hand.

The issue offered two shapes — a built-in tasks note, or "a code snippet which the user can insert on any page of their liking".
Choosing between them, and choosing a *syntax* for the second, is the decision this RFC records.
A query syntax is a **format**, so it joins the op log, the sidecar and the markdown dialect as something users write and we then owe compatibility on.
[`docs/query.md`](../query.md) documents the syntax; it does not say why this syntax.

## What we chose

A ` ```query ` fence holding one `key: value` directive per line, implicitly ANDed.
Directives shipped today: `status`, `tag`, `kind`, `since`, `text`, `sort`, `limit`.
Blank lines and `#` comments are ignored.

Results render as **live embeds** (`!((blk-XXXXXX))`), not copies, so toggling a TODO on the original is reflected everywhere it surfaces.
Query fences carry `auto_run() == true` and run on every page load, because the result depends on workspace state and not on the fence body — which makes source-hash caching wrong by construction.

Single owner: `crates/outl-exec/src/runtimes/query.rs`.
Its `dsl` module parses, its `engine` module filters and sorts against a `WorkspaceIndex`, and `QueryRuntime` returns `OutputFormat::Embeds` for the orchestrator to render.
The **same** engine is reachable structurally as `outl_exec::run_query_structured` and as `outl.query({…})` from JS, so the DSL is a surface over one engine, not a second implementation of filtering.

## Why not the alternatives

**Datalog, which is what Roam uses.**
Strictly more expressive, and that expressiveness is exactly why Roam queries are a power-user-only feature.
Datalog needs the user to know an entity/attribute schema, and outl has an op log plus a materialized tree, not a triple store.
Exposing one means publishing a queryable schema as a permanent public contract, for a problem stated as "I can't find my tasks".

**Inline `{{query: …}}`, also a Roam-ism.**
A magic token requires a new parser token in the markdown dialect, whose job is to stay standard CommonMark; a fence gets comments, multi-line bodies and editor highlighting for free.
The parser keeps `{{query: …}}` as opaque text and there are no plans to implement it — [`docs/query.md`](../query.md#relationship-to-query-) is the owner of that refusal.

**A boolean grammar with `and` / `or` / `not` and parens.**
It needs precedence rules, a real expression parser, and errors that can explain a mis-nested clause.
The AND-only line list has no precedence to get wrong and every line is independently reportable by index (`rejects_unknown_key`), and it grows without a parser rewrite — a new filter is one `enum Filter` variant plus one match arm.

**A built-in `tasks` page**, the issue's first proposal.
One hardcoded query for one shape, on a page the user cannot place or scope.
The name survives as a language alias: `tasks` and `query` resolve to the same runtime.

## The opposite direction

**What this makes worse: the implicit AND is a ceiling, and it is silent.**
"Open tasks *not* tagged `#someday`" is unexpressible today, and the user gets no error saying so — an unknown *key* is rejected with a line number, but a missing *capability* just reads as a query returning too much.
That is the failure mode this DSL trades for a syntax nobody has to learn.

**Cost of the live-view choice.**
Because fences auto-run, a page with five query blocks pays five full `WorkspaceIndex` builds from disk on every open; there is no incremental index.
Sub-second under roughly 1,000 pages, and the shards plan in [`docs/sync.md`](../sync.md#per-page-op-log-shards-for-10k-pages) is the fix before 10k.

**The mirrored case.**
The read path is safe: an embed is a reference, so a query can never duplicate or mutate the block it found.
The write path is where the asymmetry sits: the *result block* is real projected content, so a broad query grows the hosting page's `.md` with one `!((…))` line per hit.
A query narrowing from 300 hits to 3 shrinks that page again, and only the fence itself was authored by the user.

## How it cannot regress

1. **The rules.**
   [`docs/query.md`](../query.md) is the single owner of the directive table, the auto-run rule, and the refusal to implement inline `{{query: …}}`.
   No `CLAUDE.md` invariant covers this — it cannot lose data, so it does not earn one.
2. **The tests.**
   The `dsl` tests pin the accepted keys and the rejection of anything else, which is what stops a directive from being silently ignored.
   The `engine` tests pin `TODO` / `DONE` splitting, the one part of the filter that reads the markdown dialect.

## Scope

**Not covered — `or`, `not`, `between`, filter by page slug, filter by block property.**
`prop`, `page` and `group` are named as planned in [`docs/query.md` → Extensibility](../query.md#extensibility), and `prop` additionally needs the block index to expose properties.
Nothing on that list has an issue yet.

**Not covered — inline `{{query: …}}`.**
If ever wanted it is a new parser token, never a reuse of this runtime.

**Not covered — an incremental workspace index**, owned by the sharding plan in [`docs/sync.md`](../sync.md#per-page-op-log-shards-for-10k-pages) and [RFC 0137](0137-storage-scale.md).

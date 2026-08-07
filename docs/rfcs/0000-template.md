# RFC NNNN — <one line: the change, not the mechanism>

| | |
|---|---|
| **Status** | Draft \| Accepted \| Shipped \| Superseded by RFC NNNN \| Withdrawn |
| **Issue** | [#N](https://github.com/avelino/outl/issues/N) |
| **PR** | [#N](https://github.com/avelino/outl/pull/N) |
| **Date** | YYYY-MM-DD |
| **Reference doc** | the `docs/*.md` page (or per-crate `CLAUDE.md`) describing the resulting behaviour, or "none" |
| **Invariant** | root `CLAUDE.md` invariant N, or "none" |
| **Guarded by** | `test_name` (`path/to/tests.rs`), … |

## Why

The problem, in user-visible terms.
What breaks, for whom, and how they notice.
If there is a measurement, it goes here — a number from a real workspace beats an adjective.

Do not describe the solution in this section.

## What we chose

The approach, in enough detail that someone can find it in the code.
Name the single owner of the new behaviour (function, type, module).

## Why not the alternatives

One short paragraph per option considered and dropped, with the reason.
"Simpler" is not a reason on its own — say what it would have cost.

An RFC with no alternatives section is usually a decision that was never really made.

## The opposite direction

**Required.**
Do not delete this section.

State what this change makes *worse*, or what it now permits that it did not before.
For anything touching reconciliation, sync, or projection, state explicitly what happens in the mirrored case:

- If this fixes "A ran ahead of B", what happens when B ran ahead of A?
- If this makes a write safe, what happens on the read path?
- If this refuses an operation, what does the user do instead — and are they told?

This section exists because [RFC 0210](0210-md-content-outside-op-log.md) was caused by fixing one direction of a `.md` ↔ tree divergence without asking about the other.
The mirrored case deleted user content for months.
Reconciliation bugs come in pairs, and the one that destroys data is never the one being reported.

If the honest answer is "nothing gets worse", write that — but write it, so the next reader knows the question was asked.

## How it cannot regress

Two layers, both required for anything that can lose data:

1. **The invariant.**
   Which `CLAUDE.md` states the rule, and in which section.
   A rule that lives only in a code comment is not an invariant — nobody reads the file they are not editing.
2. **The tests.**
   Name them.
   They must fail if someone re-simplifies the code back to the pre-RFC behaviour, and their doc comment must say so, or a future reader will "clean them up".

## Scope

What this RFC does **not** cover, and which issue or RFC owns that instead.

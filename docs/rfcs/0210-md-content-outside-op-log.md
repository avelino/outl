# RFC 0210 — A sidecar hash match is not evidence the `.md` came from the op log

| | |
|---|---|
| **Status** | Shipped (partial — see Scope) |
| **Issue** | [#210](https://github.com/avelino/outl/issues/210), regression from [#166](https://github.com/avelino/outl/issues/166) |
| **PR** | — |
| **Date** | 2026-08-06 |
| **Reference doc** | [storage.md](../storage.md), [cli.md § `outl doctor`](../cli.md#outl-doctor) |
| **Invariant** | root `CLAUDE.md` invariant 8; `outl-md/CLAUDE.md` invariant 8 |
| **Guarded by** | `if_stale_refuses_when_the_md_carries_content_the_log_lacks`, `if_stale_still_reprojects_when_the_md_holds_no_unlogged_content`, `if_stale_ignores_whitespace_only_differences_when_deciding` (`crates/outl-actions/src/journal/tests.rs`), `a_torn_op_log_never_lets_repair_overwrite_a_good_md` (`crates/outl-cli/src/cmd/doctor/tests/safety.rs`) |

## Why

On a real workspace — 2,560 pages, 213,859 ops, in daily use since June — **233 pages held 1,426 lines of content that existed in no op**.

The content was not scratch: infrastructure learnings with run ids, operational briefings, root-cause notes spanning 2022 to 2026.
It read correctly on disk, in the editor, and in any `grep`.
It had simply never been recorded as an `Op`.

Three consequences, in increasing severity:

- **It never reached another device** — peers exchange ops, not files, so all 1,426 lines existed on exactly one machine, with no second copy for a user who trusts the sync story.
- **Nothing surfaced it** — not `doctor`, not any consistency check, not one log line, because `sidecar.last_synced_hash` agreed with the bytes on disk, and that agreement is what every downstream check tests.
- **Re-projection deleted it and reported success** — `outl doctor --repair` printed `708 fixed` while removing content from 233 pages, then rebuilt each sidecar from the same render, so afterwards nothing could tell the page had ever held more.

The deletion was not limited to `--repair`.
`apply_page_md_with_sidecar_if_stale` runs on every GUI open path (`open_page_by_slug`, `open_journal_for`, `open_today_journal`, `open_ref`), so opening the page in the desktop or mobile app was enough.

The root cause is a conflated question.
`sidecar.last_synced_hash == file_hash(disk)` answers *did outl write these bytes last?*
It was read as answering *did these bytes come from the op log?*
Every page in the state above answers yes to the first and no to the second.

## What we chose

`outl_actions::content_lines_missing_from(disk, rendered) -> Vec<String>` is the single owner of the verdict "would re-projecting delete something?".
`apply_page_md_with_sidecar_if_stale` calls it after the hash gate passes and returns `ActionError::PageMarkdownAheadOfLog { path, lines, sample }` instead of writing.

Two details that carry weight:

**It compares a multiset of trimmed non-blank lines, not a diff.**
A line the renderer merely *moved* is not at risk, and an LCS diff reports it as unique to disk.
On the measured workspace that is the difference between flagging 616 pages and the 233 that genuinely hold unlogged content.

**Whitespace-only drift is ignored on purpose.**
The renderer's trailing-newline behaviour changed between releases, so a large share of "stale" pages differ from the log by exactly that.
Treating it as content would strand every genuine re-projection behind noise, and a guard that fires constantly gets disabled.

`outl doctor` calls the same function in its read-only listing, so the listing can no longer offer a repair the `--repair` pass then refuses — the "announced before they run" invariant in `outl-cli/CLAUDE.md`.

## Why not the alternatives

**Return `Ok(None)` and skip the write silently.**
The cheapest change, and it fails the test this project cares most about: the user learns nothing.
A page quietly not re-projecting looks identical to a page that needed nothing, so the 1,426 lines stay unsynced and undiscovered.
Silence is the defect, not the write.

**Compare block ids from the sidecar against the tree instead of text.**
Structurally cleaner, and blind to the actual case: the sidecar in this state has *already* been rewritten to describe the file, so its ids and the tree agree while the text does not.
The evidence lives in the content, which is why the check reads content.

**Make `reconcile_md` correct and trust the hash again.**
Right, and not sufficient.
It fixes the producer for the future while leaving every workspace already in this state to be emptied by the next page open.
It also leaves the false inference — "hash matches, therefore the log holds it" — in place for the next caller to repeat.
The producer fix is still needed — see Scope.

**Refuse on any hash-faithful page whose render differs.**
This is the pre-#166 behaviour, and it re-breaks #166: a peer's ops land, the `.md` is never refreshed, the page renders empty.
The point of the multiset check is to separate those two cases rather than choosing which one to lose.

## The opposite direction

**What this makes worse:** a page holding unlogged content now refuses to re-project, so a genuine tree-ahead update to that same page does not reach the `.md` either.
That is deliberate — a stale view is recoverable, deleted content is not — but the user is stuck until `outl reconcile` runs, and they only learn this from `doctor` or an error surfaced by the client.
`doctor` names the count and one sample line for exactly that reason.

**The mirrored case, stated explicitly:** this RFC fixes "`.md` ran ahead of the tree" (content deleted).
The mirror is "tree ran ahead of the `.md`" (content hidden), which is #166, and it stays fixed: `if_stale_still_reprojects_when_the_md_holds_no_unlogged_content` pins it.
Both directions are now pinned by a test, and neither can be reintroduced by simplifying the other away.

**One precedence trap found while implementing this.**
A torn op log replays a truncated tree, which makes *every* page look like it holds unlogged content.
The first version of this guard therefore hijacked the report on a damaged log, and the message telling the user how to recover the log disappeared — caught by the existing `a_torn_op_log_never_lets_repair_overwrite_a_good_md`.
The check now stands down when `OpLogHealth::is_compromised()`, so a damaged log is reported as a damaged log.
That test was written for a different defect and caught this one; it is the strongest argument in this RFC for naming tests as a required section.

## How it cannot regress

1. **Invariants.**
   Root `CLAUDE.md` invariant 8 states the rule for consumers (never overwrite a `.md` on the hash gate alone) and carries the *why*, so it cannot be argued away as paranoia.
   `outl-md/CLAUDE.md` invariant 8 states it for the producer: `last_synced_hash` may only advance over content the same call emitted ops for.
   Three anti-patterns in the root `CLAUDE.md` name the specific mistakes.

2. **Tests.**
   The four in **Guarded by** above.
   The three in `outl-actions` cover refusal, the still-must-reproject case, and whitespace tolerance; the `outl-cli` one pins the precedence order against a damaged log.
   The root `CLAUDE.md` says outright that they exist to fail if the gate is re-simplified, and must not be relaxed.

## Scope

**Not covered — the producer.**
How a page reaches this state is still open on [#210](https://github.com/avelino/outl/issues/210).
The suspect is `reconcile_md` rewriting the sidecar to agree with a file it did not fully emit ops for (`crates/outl-md/src/reconcile.rs:93,155,261`).
This RFC prevents the deletion; it does not stop the state from being created.

**Not covered — recovering existing content.**
The 1,426 lines on the measured workspace are still outside the log.
`outl reconcile` owns the `.md → tree` direction, and nothing today walks a user through 233 pages of it.

**Not covered — volume guards.**
Matching level 3 trashes 1 block and 5,000 identically (`crates/outl-md/src/matching.rs:295`), and re-projection deleted across 233 files in one pass with no threshold and no count in the output.
Tracked on the same issue.

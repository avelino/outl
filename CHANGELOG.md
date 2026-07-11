# Changelog

All notable changes to outl are documented here.
Format inspired by [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); the project uses [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- **Multi-block batch operations, on the TUI and desktop (issue #23).**
  The TUI's Visual mode (`V`) gains **reorder**: `Alt+↑` / `Alt+↓` drag the whole selection among its siblings (mirror of the single-block `Alt`+arrows in Normal), alongside the existing range delete / indent / outdent / yank.
  On the desktop, multi-select no longer requires vim mode: **`Shift+↓` / `Shift+↑`** start and grow a contiguous block selection from anywhere (the non-vim entry), and a floating **batch toolbar** appears — `N selected` plus Indent, Outdent, Move up, Move down, Delete, and Done — so the range ops are reachable by mouse instead of only by chord.
  The toolbar fires the **same** `action-handlers` the keyboard does, so button and chord can't drift; only the toolbar's Delete confirms before erasing a range with nested children (the keyboard delete and the TUI erase without a prompt, matching vim).
  The range reorder (`Cmd/Ctrl+Shift+↑/↓` in Visual) loops the existing per-block move action, walking bottom-up for move-down so a block never drags over its own not-yet-moved neighbour; the selection follows because block ids are stable across the re-render.
  All four new bindings live in the shared `outl-shortcuts` catalog (`SelectRange{Down,Up}`, `MoveVisualRange{Up,Down}`), so a future client inherits them.
- **Template engine — reusable block structures and callable code blocks (issue #146).**
  Any page becomes a template the moment it gets a `template:: <name>` property; the page's outline is the template body, so templates are searchable, have backlinks, and sync like any other page — no special folder, no file-based config.
  Two invocation modes.
  **Structural** (`/template <name>` in the TUI, `outl template apply <name> --page <slug>` on the CLI, `outl_template_apply` over MCP) deep-copies the template's subtree under the target block, minting fresh `NodeId`s and op-log entries, and substitutes the built-in variables `{{date}}`, `{{today}}`, `{{yesterday}}`, `{{tomorrow}}`, `{{page}}`, and `{{time}}` in the block text.
  **Callable** (a ` ```call:<name> ` fence, run with `gx` in the TUI or the Run action on desktop) resolves the named template's code block, injects the `params::` declared on the call block, and executes it through the existing `outl-exec` runtimes — Roam's `{{roam/render}}` without a ClojureScript runtime.
  Callable execution lives once in `outl_actions::run_callable_block` and is intercepted inside the shared `run_code_block` action, so the desktop and mobile Run paths get `call:` fences for free instead of erroring "no runtime for `call:<name>`".
  On the GUI clients a `call:<name>` fence now renders as a proper code block (with a language chip and Run button) — the shared `detectFence` info-string pattern accepts the `:` so the block is no longer left as raw ```` ``` ```` text, and its `key: value` params are syntax-highlighted (as YAML) instead of rendering flat.
  Finishing an edit on a `call:<name>` block re-runs it automatically, so the `> **result:**` reflects the freshly-typed params without a manual `gx` / Run — on the TUI (Insert commit) and both GUI clients (the shared `edit_block` command).
  **Every template page shows where it was used.** The template page's backlinks panel now lists every block that rendered it (a `call:<name>` fence) or instantiated it (`from-template:: <slug>`), so you jump from a template to its call sites with no hand-written `[[link]]` — the matcher reads the fence and the provenance property directly, not just plain `[[refs]]`.
  **The daily journal is now a template too.** `outl init` creates a `templates/journal` page (`template:: journal`) instead of a `templates/journal.md` file, and opening a fresh daily note stamps that template automatically — the built-in variables resolve against the daily's date. Existing customized `templates/journal.md` bodies migrate into the page on `init` (best-effort).
  Callable params are injected as JSON (`serde_json`), so a value containing a quote can't break — or inject into — the generated program, and the language is canonicalized so `py`/`python3`/`node` aliases still receive the params prelude. Built-in date/time tokens resolve through the workspace clock (honouring `[calendar] timezone`), matching the journal date instead of reading UTC in containers.
  The engine lives once in `outl_actions::template` and every client wraps it (TUI, CLI, MCP), so the semantics stay identical across surfaces.
- **Paste with formatting now brings rich clipboard formatting across (bold, italic, links, lists) on the GUI clients.**
  Copying a formatted message — a Slack post, a Google Doc paragraph, a Notion block, a Gmail draft — puts the bold/italic/links/lists on the clipboard's `text/html` flavour; the `text/plain` flavour is stripped of them.
  The desktop and mobile paste used to read only `text/plain`, so a pasted Slack message arrived flat.
  It now reads `text/html` first and converts it to outl markdown (via **Turndown**, tuned for the outl dialect: `*italic*` not `_italic_`, `- ` bullets, `~~strike~~`, and Slack `:emoji:` kept from the image alt text), then routes it through the same paste pipeline — so the formatting and the bullet structure survive.
  Google Docs (and other editors that encode weight as inline CSS) are handled too: a `font-weight:700` span becomes `**bold**`, and the `<b style="font-weight:normal">` wrapper Docs wraps the whole payload in no longer bolds the entire block.
  Plain text with no richer HTML behaves exactly as before.
  The converter lives once in `@outl/shared/paste` (`htmlToOutlMarkdown`) so both GUI clients stay identical.
- **Paste with / without formatting, with explicit chords per client.**
  "With formatting" routes the clipboard through the conversion pipeline: outline syntax (Roam `{{[[TODO]]}}`, GitHub `- [ ]`, Logseq) is normalized to the outl dialect, and **plain multi-paragraph text is split into one block per paragraph** — a pasted chat reply or email lands as a readable outline instead of one wall-of-text block (blank line = paragraph break; soft line wraps stay in one block).
  "Without formatting" splices the raw clipboard text into the current block, no conversion, no splitting.
  Desktop: `Cmd/Ctrl+V` = with formatting, `Cmd/Ctrl+Shift+V` = without.
  TUI: `p` = with formatting, `Shift+P` = without (both read the OS clipboard now; the old `p`/`P` yank-register paste is folded into this since copy mirrors the register to the clipboard).
  Mobile: paste is always with formatting.
- **Copy a block selection as clean markdown to the OS clipboard, in every client (issue #114).**
  Copying out of outl used to be a mess — selecting a block in the TUI with the terminal's mouse copied the on-screen tree guides (`│ `), bullets, and fold markers, so pasting elsewhere produced garbage.
  Now every yank/copy writes the **canonical outl markdown** for the selection (each block plus its subtree) to the clipboard, so it re-pastes into outl as the same tree and reads as a tidy bullet list anywhere else.
  TUI: `yy` / `Y` / Visual `y` write the markdown to the clipboard via `arboard` **and** an OSC 52 escape, so it reaches the clipboard over SSH, inside tmux, and in Chrome OS **Crostini** where `arboard` has no display server (the in-app yank register that `p`/`P` reads is still filled too).
  Desktop: `Y` / Visual `y` copy the selection as markdown via `navigator.clipboard`.
  Mobile: the long-press **Copy** action copies the block and its subtree as markdown instead of a single block's raw text.
  A Visual range spanning a parent and one of its children no longer duplicates the child — the shared serializer drops any block whose ancestor is also selected.
  This is the copy-out inverse of the existing paste-in conversion, so the two round-trip; the serializer lives once in `outl_actions::copy_markdown` and every client wraps it.
- **Opt-in mouse support in the TUI — new `[tui] mouse_capture` config key.**
  Set `[tui] mouse_capture = true` in `~/.config/outl/config.toml` and the TUI captures the mouse: the scroll wheel moves the selection, a click selects the block under the pointer, and a drag selects a range that is copied to the clipboard as markdown on release.
  Default `false`, and deliberately so — capturing the mouse disables the terminal's own text selection (selecting a URL, copying a single word), which is muscle memory for many terminal users.
  The keyboard yank copies markdown to the clipboard regardless of this flag.

### Changed

- **P2P sync now defaults to outl's dedicated relay (`use1-1.relay.avelino.outl.iroh.link`) instead of the shared n0 public pool.**
  The relay only ever sees end-to-end-encrypted bytes (never your notes), but it *can* observe coordination metadata — which two devices sync, and when.
  Defaulting to a dedicated, outl-scoped relay endpoint (hosted on n0 infra under our `*.iroh.link` namespace) is the first step toward a fully outl-owned relay; n0's shared relays remain the documented fallback (a malformed `[sync] relay_url` degrades to them rather than failing the bind).
  No action needed — a device with an empty / omitted `[sync] relay_url` picks it up automatically. Point `relay_url` at any `iroh-relay` to override. See `docs/relay.md` (the vanity `relay.outl.app` name is on the roadmap, pending TLS).

### Fixed

- **Template footguns from the issue #146 release audit.**
  Three sharp edges in the template engine (issue #146) are fixed.
  **Callable vs structural dispatch now keys off the presence of a runnable code block**, not on whether `params::` is declared — a callable template with a code block but no `params::` used to be misrouted as structural, so its ` ```lang…``` ` fence got deep-copied as literal text instead of executing; it now runs (with an empty `params`).
  **Duplicate template names are visible** — when two pages share a `template:: <name>`, resolution still picks the first in tree order but now logs a `tracing::warn!`, and `list_templates` flags the collision on each `TemplateEntry` (`duplicate`) so a client can surface it.
  **Plugin-instantiated templates honour the target page's date** — the host derived `{{date}}` from *today* even on a journal page (`page_date: None`); it now derives it from the target slug, matching the CLI/TUI path.
- **P2P sync no longer reports a false "sync ok" that silently drops a device's edits.**
  A desktop-initiated delta-sync logged `catch-up: sync ok` as soon as it finished *writing* its push — never confirming the peer durably *ingested* it.
  Over a lossy desktop → mobile path (a backgrounded iPhone / carrier NAT), the connection could tear down cleanly for the initiator while the mobile never persisted the pushed ops, so a page edited on the desktop stayed empty on the phone even though sync claimed success.
  The responder already closes the stream with a `done` sentinel **only after** a durable ingest; the initiator now **requires** that sentinel before reporting success (a lost close on an otherwise-successful ingest just costs a harmless, deduped re-push).
  Regression: `initiator_reports_failure_when_responder_never_confirms_ingest`.
- **Sync connect no longer stalls ~30s on a stale peer address, and self-heals when a device moves networks.**
  iroh 1.0.0's QUIC multipath opens a path to every stored address at once and wedges ~30s on a dead one; a peer's old on-LAN IP (still in `peers.json` after it moved Wi-Fi / went cellular) passed the subnet filter and stalled every catch-up tick.
  Each connect attempt is now bounded by a timeout, and a stalled/failed direct dial falls back to a **bare-node-id** dial so iroh's relay + discovery resolves the peer's *current* address instead of retrying the dead one forever.
  On top of that, when a peer dials *in* directly, outl reads the live socket off the connection and rewrites the stored address to it (dropping the stale one), so the next outbound dial uses the fresh route with no re-pair — the stored address self-heals the moment the peer reconnects.
- **Mobile stops flip-flopping a page between two devices' states on the sync poll.**
  The mobile's routine reload (every ~3s) ran the orphan-`.md` reconcile + desync-recovery **inline** — operations that *mutate* the op log (md → ops). On a page being edited on two devices while sync ingested peer ops, the desync-recovery false-positived on the racing read and minted fresh ops each poll, so the page oscillated between the desktop's and the phone's versions (and briefly flashed an empty "0 ops" state).
  Reconcile/recovery is a **boot** concern (a stable moment, no concurrent ingest); iroh peers ship *ops*, not `.md`, so a routine reload only needs to re-materialize the op log. The reload is now a pure re-read — orphan `.md` recovery still runs once at boot — and the reload no longer clobbers real content with a transient empty read.
- **Callable-template results stop churning the op log and oscillating across devices.**
  The `> **result:**` subtree was deleted and recreated on every run with fresh node ids, so two devices running the same `call:` block fought a delete/recreate war (each deleting the other's result), bloating the op log into the thousands and flip-flopping the page between the two devices' outputs.
  The result now uses a **deterministic node id derived from the call block** and updates in place, so re-runs are idempotent and two devices converge on one result (last write wins per line) instead of competing subtrees.

- **iroh sync survives restrictive networks — custom-CA proxies and post-VPN stale peer addrs (issue #133).**
  Two blockers that stopped Mac ↔ iOS sync on corporate / VPN networks are fixed in code.
  **Relay TLS with a custom root CA:** a network with a TLS-inspection proxy (its root CA installed in the OS keychain) had every relay handshake rejected with `invalid peer certificate: UnknownIssuer`, because iroh trusted only Mozilla's bundled roots — not the OS trust store that macOS / `curl` / Safari already honour.
  outl now delegates relay-TLS validation to the OS keychain (iroh's `platform-verifier` + `CaTlsConfig::system()`, wired once in `bind::n0_builder_ipv4_only`), so a keychain-trusted proxy cert is accepted.
  **Stale VPN/tunnel IPs after pairing:** a device paired while on a VPN captured its tunnel IPs (`10.x`, `100.x` CGNAT, a public WAN addr) into `peers.json` alongside the real LAN address, and iroh 1.0.0's multipath opened a path to each — stalling even same-WiFi direct sync on the dead paths (`MultipathNotNegotiated`) with the reachable `192.168.x` addr right there.
  A dial now keeps only the direct IPv4 addresses that share a subnet with a local interface, dropping unreachable tunnel IPs before they can stall the connect (the relay still covers genuine cross-network peers).
  A third failure mode — a proxy that blocks the relay's WebSocket `Upgrade` and returns `502` — is environmental (iroh 1.0.0 has no non-WebSocket relay transport); the workaround is a self-hosted `[sync] relay_url`, now documented under `docs/relay.md` → "Troubleshooting: restrictive networks".
- **Underscores inside a word no longer render as italics.**
  The inline tokenizer paired any `_…_` as emphasis, so pasted identifiers like `chamados_chat`, `inc_lag1`, `prod.ml_atendimento`, or `databricks_2_train` rendered half-slanted.
  outl now follows CommonMark: `_` only opens or closes emphasis at a word boundary, never intra-word (`*` stays the intra-word marker).
  A standalone `_italic_` still works.
- **`o` / new-line no longer errors with "block … is not in the tree" after a background reload.**
  On the desktop, a peer-driven reload (`peer-ops-changed`) replaced the outline without clearing the editing / selection cursor, so a block being edited could keep an id the reload had re-materialized or dropped — the next edit or new block then hit "block … is not in the tree".
  The reload now prunes a stale `editingBlockId` / `selectedBlockId` against the fresh outline, and `create_block` (desktop + mobile) falls back to appending at the page end if its anchor is gone.
- **Pasting into a freshly-created empty block no longer errors with "block … is not in the tree".**
  A block created with `o` (or a new line) carries only an `Op::Create`, no `Op::Edit`, so it has no materialized text yet — and the caret-paste path guarded the host's existence on `block_text`, which returns `None` for a text-less block, so pasting into a brand-new empty block was rejected as if the block didn't exist.
  The paste (with **and** without formatting) now checks the tree for the block and treats a missing text as empty, so it grafts into the new block on desktop and mobile.
- **Desktop error messages surface as a top-right toast instead of a hidden bottom banner.**
  The error surface used to render a full-width banner at the base of the outline, where the fixed bottom-left chrome cluster painted over its left edge — the message was half-covered.
  Errors now appear as a floating toast in the top-right notification corner, above every chrome element, with nothing overlapping it.
- **Journal date and status-line clock honour a configured timezone — new `[calendar] timezone` config key.**
  The journal's "today" and the TUI clock used to call `chrono::Local::now()`, which trusts the operating system's local timezone.
  In containers and Chrome OS **Crostini** the OS clock runs in UTC regardless of where the user is, so the date landed on the wrong day near midnight and the clock read an hour off (issue #107).
  A user can now set `[calendar] timezone = "Europe/London"` (any IANA name) in `~/.config/outl/config.toml`; the journal date and clock resolve in that zone, DST-aware.
  The fix is opt-in: with no timezone configured the clock stays on the OS local timezone, so nothing changes for a normally configured machine.
  Internally this is the new `outl_actions::clock` module (`init` / `now_local` / `today`, backed by `chrono-tz`); every client calls `clock::init` once at boot and every "today" routes through it (`page::today` delegates), so there is a single source of truth for the user's wall clock.

### Added

- **JavaScript plugin system (`outl-plugins`), shared by every client.**
  Plugins are bundled JavaScript described by a `plugin.json` manifest; a plugin written once runs on every outl client because it talks to the new `outl-plugins` crate, never to anything client-specific.
  The engine is **Boa** (pure-Rust, runs on iOS — no JIT, reused from `outl-exec`) behind a `PluginEngine` trait so it can move to QuickJS later only if a measured blocker appears.
  Execution is **describe → apply**: the JS side reads a pre-computed `ReadModel` and emits `HostIntent`s; the host drains them through `outl-actions` → `Workspace::apply`, so the op log stays the single source of truth and `.md` stays 100% clean.
  Live capabilities: `op-hook` (`onOp`), `slash-command`, `keybinding`, `config-schema` (read), `toolbar-button`, `ui-render`, and `content-transformer:text` / `:rich`; `sync-transport` is core-ready (convergence tested) but no client polls it on a timer yet.
  `keybinding` fires a bound chord from the **TUI** (Normal mode, single + two-chord, never overriding a native binding) and the **desktop** (a native binding always wins); `toolbar-button` renders a chrome button on desktop + mobile and surfaces the command in the TUI slash menu; `content-transformer` (`ctx.content.register(lang, fn)`) renders a fenced block as text on every read surface (`:text`, inline in the TUI) or as HTML in a sandboxed iframe on the GUIs (`:rich`).
  New host namespaces: `ctx.net.fetch` (blocking HTTP gated by `network:<domain>`; a denied domain returns `{ ok: false, error }` rather than throwing), `ctx.storage.{get,set,delete}` (per-plugin local KV gated by `storage:local`, stored at `<workspace>/.outl/plugins/<id>/storage.json`, deliberately outside the op log so it never converges), and `ctx.sync.register({ push, pull })`.
  A query engine plugs in as a `content-transformer` for the `query` fence — there is no separate `query-provider` capability, and inline `{{query}}` waits on a markdown token the parser defers.
  A capability the current client can't honor loads partially with a warning (never a crash); every host call is gated against the user-approved permission set (`read-page`/`write-page`/`read-op-log`/`submit-op`/`storage:local`/`network:<domain>`, with bare `network:*` rejected).
  Anti-loop is structural: `PluginHost` tracks how far into the op log it has dispatched, so a plugin's own ops never re-trigger its hooks.
  A runaway plugin can't wedge the engine either: Boa runs under `RuntimeLimits` (loop-iteration cap ~20M, recursion cap ~2000, stack cap), so an infinite loop or unbounded recursion surfaces as a JS error instead of a hung thread.
  Wired into **TUI** (plugin commands in the slash menu, `onOp` after each mutation) and the **CLI** (`outl plugin list / install / run / enable / disable / remove`, the last aliased `uninstall` / `rm`); the desktop/mobile wiring runs the host on a dedicated thread (the Boa context is `!Send`).
  Distribution day-zero: install from a local directory (`github:` source to follow), a `bundleHash` revalidated on every load, a per-workspace `installed.json` lockfile freezing the approved permissions, and a static `registry.json` index (the "store").
  Authors get `@outl/plugin-sdk` (typed `definePlugin` + host API) and two working examples: `examples/todo-archiver` (archives DONE blocks to a configurable page) and `examples/confetti` (throws a confetti burst when a block is marked DONE).
  The `ui-render` capability lets a plugin hand the GUI clients (desktop + mobile) a chunk of author-written HTML/JS via `ctx.ui.render(html)`, which they run in a **sandboxed iframe** (`sandbox="allow-scripts"`, no same-origin — isolated from the app DOM) as an ephemeral full-screen overlay.
  The host stays agnostic: it only transports the string the plugin produced, so the visual is the author's creativity, not a fixed catalog of effects.
  The TUI/CLI drop `ui-render` payloads (no webview); the op-hook still fires.
  New `outl_actions::block::move_under` (re-parent a block under an arbitrary page/block) backs the plugin `Move` intent.
  See `docs/plugins.md`, `docs/plugin-api.md`, and the manifest schema at `docs/schemas/plugin-v1.json`.
- **`:shortcode:` emoji syntax + autocomplete across every client.**
  The outl inline dialect now recognises GitHub-style gemoji shortcodes (`:tada:`, `:rocket:`, `:smile_cat:`, `:+1:`, `:100:`) and renders them as the unicode glyph (🎉, 🚀, 😸, 👍, 💯) on every read surface.
  The catalog is the [`emojis`](https://crates.io/crates/emojis) crate (Unicode CLDR + GitHub aliases, ~1800 shortcodes) so `outl_md::emoji::search` is the one ranking source TUI, mobile, and desktop share through a single `outl_emoji_search` Tauri command — no parallel index on the JS side.
  **Disk form is always the shortcode literal** (`:tada:`, never 🎉) so `.md` files stay greppable, diffable, font-independent, and safe across iCloud / Syncthing.
  The parser only tokenises `:foo:` when the catalog recognises `foo`; unknown runs (`:notarealemoji:`, `meeting at 14:00 :`) stay plain.
  URL boundaries fall out for free — the strict `[a-z0-9_+-]+` shape + catalog gate reject `https://example.com:8080/api`, `mailto:foo@bar.com`, and `git@github.com:avelino/outl.git` without a look-behind pass.
  Typing `:roc` inside any block opens a popup with the top eight matches (`outl_md::emoji::search`, exact → prefix → substring, shorter shortcodes win ties); `Tab`/`Enter` commits the canonical `:rocket:` form into the buffer.
  Wired into `outl-tui` (`AutocompleteKind::Emoji` + the existing overlay machinery), `outl-mobile` (UIKit chip strip via `buildEmojiShowMessage`), and `outl-desktop` (floating `EmojiSuggestPopup` under the textarea, parallel to `RefSuggestPopup`).
  The shared `@outl/shared/autocomplete::detectEmojiContext` + `applyEmojiSuggestion` own the trigger detection and splice so the three GUI surfaces stay byte-identical.
  See `docs/markdown-format.md` § "Emoji shortcodes" for the full dialect contract.
- **`@` mention autocomplete** — typing `@` in any block opens a person picker filtered to pages with the `type:: person` page-level property, fuzzy-matched against the typed name.
  Accepting a candidate inserts `[[@name]]`, a regular wikilink whose `@` is the link affordance only (page identity stays clean, slug has no `@`).
  Composite names like `@Thiago Avelino` work because the autocomplete query allows spaces.
  A "create new" candidate appears when the typed query doesn't match any existing person.
  Accepting it materialises a fresh `pages/<slug>.md` with `type:: person` already set, so the next mention of the same name surfaces it without manual property editing.
  Wired identically in `outl-tui`, `outl-desktop`, and `outl-mobile`.
  The shared `@outl/shared/autocomplete` library owns the trigger detection and the create-new helper for the GUI clients.
- **`type::` page-level property** — surfaced on `outl_md::WorkspaceIndex::PageEntry.page_type` and `outl_actions::PageMeta.page_type`.
  New filter `WorkspaceIndex::pages_by_type(t)` and consumer `outl_actions::page::search_persons(ws, query)` rank `type:: person` pages for the `@` mention popup.
  `type::` is just one of many user-facing page properties (`title::`, `icon::`, `pinned::`, `role::`, anything custom) and all of them now reach the workspace tree (see "Fixed" below).
- **`ref-projection-failed` Tauri event** — emitted by the desktop and mobile clients when `open_ref` resolved a target (the page is in the op log) but writing the resulting `.md` + sidecar failed.
  Frontend can listen via `onRefProjectionFailed` (desktop) and surface a toast so the user knows the link they just inserted isn't visible to peers yet.
  The op log retry on the next save / orphan scan is still automatic.

### Fixed

- **TUI now word-wraps block text to the pane width** ([#99](https://github.com/avelino/outl/issues/99)).
  Typing past the right edge of the terminal used to run a block off-screen instead of flowing onto the next visual row — terminals don't reflow on their own, and the outline deliberately avoided ratatui's `Paragraph::wrap` because that expands lines *after* layout and would desync the `selected_line` scroll index.
  The outline now pre-wraps itself (`outl-tui` `view::wrap::push_wrapped`): wrapped rows are emitted up front so the scroll index stays honest, the first visual row keeps the bullet/fold marker, continuations re-indent under the text column, and the `│ ` indent rails repeat top to bottom.
  Wrapping runs on the already-styled spans (post-tokenization), so a break never splits a `**bold**` / `[[ref]]` token back into its literal markers, and wide glyphs (CJK, emoji) count as two cells.
  The block being edited (Insert) or selected in Normal mode stays on one line so the cursor column keeps matching the source bytes.
- **Page-level properties now reach the workspace tree.**
  The reconcile pipeline previously emitted `Op::SetProp` only for block-nested properties.
  Anything written at the top of a `.md` (above the first `-` bullet) — `title::`, `icon::`, `pinned::`, `type::`, custom keys — lived only on disk.
  The workspace CRDT never learned about them, so any consumer reading via `workspace.tree().property(page_id, …)` (desktop sidebar, mobile picker, `outl_actions::search_persons`) silently disagreed with the rendered markdown.
  The TUI hid the bug because its `WorkspaceIndex`-backed surfaces parse `.md` straight from disk.
  Cross-client divergence on every workspace populated outside the in-app picker (fixtures, vim users, Logseq/Roam imports, peers via iCloud) was the result.
  Fix: `outl_md::diff::diff_to_ops_with_page_props` emits `Op::SetProp` on the page root for every page-level property in the parsed AST.
  `outl_md::reconcile::reconcile_md` calls it on every reconcile pass.
- **Page root now materialises in the tree.**
  Pages authored externally never received an `Op::Create`, only the blocks under them did.
  Combined with the CRDT contract that `Op::Move` on a node without a preceding `Op::Create` is a no-op (an intentional design for peer-sync ordering), this left the page node as a ghost: present as `parent` of its blocks but absent from `children_of(NodeId::root())`.
  `list_all_pages`, `search_persons`, and the sidebar all skipped externally-authored pages silently.
  Fix: `outl_md::reconcile::ensure_page_root_in_tree` emits `Op::Create` when the page node is absent from `self.nodes`, or `Op::Move` when it exists at the wrong parent, plus `Op::SetProp` for `page-slug` / `page-kind`.
  Idempotent: 0 ops emitted on pages that are already materialised.
- **`open_ref` regenerates `.md` after creating a page.**
  Both desktop and mobile previously left newly-created pages on the op log only.
  The `pages/<slug>.md` projection never landed on disk until something else triggered `apply_page_md_with_sidecar` on that page.
  `WorkspaceIndex` (which parses `.md` from disk) disagreed with the tree CRDT silently, and a peer pulling the workspace via iCloud would never see the page at all.
  Fix: both clients now call `apply_page_md_with_sidecar` immediately after the `open_or_create_by_ref` mutation; failures emit `ref-projection-failed`.
- **`open_or_create_by_ref` no longer drops the `@` arm via slug normalisation.**
  `slugify("@avelino")` strips the `@` and returns `"avelino"`.
  The generic `find_by_slug(slugify(target))` branch used to run before the `@` arm, so a pre-existing `pages/avelino.md` (created before this feature, or by an external editor without `type:: person`) resolved via the generic path and returned early, never marking the page as a person.
  Fix: the `@` arm runs first and idempotently sets `type:: person` on every resolution, even when the page already existed.
- **`reconcile_md` reads the sidecar once.**
  The short-circuit check used to re-read the sidecar file separately from the diff inputs, racy if another process rewrote the sidecar between the two reads.
  Fix: single read, both consumers share the result.
- **Background-thread reconcile after open.**
  Opening a workspace used to block on a synchronous `reconcile_md` pass across every legacy page.
  With many pages, the first paint waited tens of seconds.
  Fix: `outl-desktop::workspace_open::spawn_background_reconcile` runs the orphan reconcile on a separate thread, locks the workspace per page (released between iterations), and emits `workspace-reconciled` on completion.
  Today's journal opens immediately; legacy pages materialise behind the scenes.

### Migration

- **`pipeline_version` in the sidecar drives forward-compatible re-reconciles.**
  The first boot on an upgraded binary scans every `pages/<slug>.outl` and re-runs `reconcile_md` on any sidecar whose `pipeline_version` is below the binary's `CURRENT_PIPELINE_VERSION`.
  Idempotent: the pipeline emits the same `Op::Create` / `Op::SetProp` ops that would have been emitted on first ingestion of the `.md`, the CRDT deduplicates by LWW, and the sidecar persists the bumped version.
  Two clients opening the same legacy workspace at the same time will each run their own reconcile (each actor owns its `ops-<actor>.jsonl`).
  The log inflates by roughly 2× the page-root ops once per legacy page per device.
  Acceptable for a one-shot migration: the CRDT converges deterministically.
  Subsequent boots skip the page via the `last_synced_hash` + `pipeline_version` short-circuit.



**Desktop client ships.**

`outl-desktop` (Tauri 2 + Solid + Tailwind) lands as the third client alongside `outl-tui` and `outl-mobile`, sharing the same `outl-actions` surface, the same op log, and the same workspace on disk.
Three new Rust crates (`outl-config`, `outl-theme`, `outl-shortcuts`) extract per-client config + palette + chord catalog out of the TUI so both clients converge on one source of truth; `@outl/shared` (`crates/outl-frontend-shared`) does the same for the Solid + DTO frontend code mobile and desktop both need.

The MINOR bump is the desktop addition; CRDT, sidecar, and existing CLI/TUI/mobile contracts are unchanged.

### Added

- **`outl-desktop`** — Tauri 2 client for macOS, Linux, Windows. 2-pane layout (Sidebar / OutlineView with inline backlinks at the bottom, mirroring the TUI), mini-calendar + pinned + recent in the sidebar, `outl-exec` code-block execution, cross-platform FS watcher (`notify`) that emits `peer-ops-changed` so the frontend reloads when iCloud / Syncthing / shared FS drops a peer's `ops-*.jsonl`. Distributed as a **universal macOS dmg** (arm64 + x86_64 lipo-merged) via `brew install --cask outl-desktop-beta`.
- **`outl-config`** — shared TOML config at `~/.config/outl/config.toml` (XDG on every OS — Windows routes through `dirs::config_dir()` to `%APPDATA%`). Read by TUI / CLI / desktop through the same `outl_config::load()` so a theme set in the desktop's Settings modal lights up in the TUI on the next launch.
- **`outl-theme`** — palette catalog with the seven existing presets (`outl`, `default-dark`, `light`, `dracula`, `solarized-dark`, `nord`, `monokai`). TUI derives its `Theme::from_palette` from here; desktop ships the palette over the Tauri wire and writes CSS custom properties.
- **`outl-shortcuts`** — `(chord → action)` catalog every client consumes. TUI translates `crossterm::KeyEvent` → `Chord`; desktop's `KeyboardEvent` adapter does the same. One binding change lights up on both clients.
- **`outl-frontend-shared`** (`@outl/shared`) — pure TS+Solid lib with the `MarkdownInline` renderer, paste / autocomplete helpers, DTO types, and the typed `invoke<T>()` wrappers every client uses. Mobile already consumed these locally; promoted in this release.
- **`PageMeta.pinned`** — the `pinned:: true` page property is now surfaced on `PageMeta` (matching `outl-md::index::PageEntry.pinned` exactly so the two never drift on which literals count as truthy). Sidebars on TUI + desktop pick it up.
- **Backlinks navigable on desktop** — `j`/`k` extends past the outline's last block into the inline backlinks section; `Enter` opens the source page and parks the cursor on the referencing block. Mouse click does the same. Mirrors what the TUI already did.
- **Workspace path fallback for `outl` with no args** — `outl_config::load().workspace.last` is consulted between `--workspace <DIR>` and the cwd, so the TUI lands on whatever workspace the desktop opened last with no flag.

### Changed

- **TUI sidebar chord** — `\` → `Ctrl+E` (mirroring desktop's `Cmd+Shift+E`, the VS Code "Show Explorer" convention).
- **TUI backlinks chord** — `B` → `Ctrl+B` (mirroring desktop's `Cmd+Shift+B`; we kept `Cmd+B` reserved for the universal markdown "bold" chord in Insert mode).
- **Sidebar + backlinks default to hidden** on the desktop now, matching the TUI's editor-hero defaults. Users opt the panes in with the chord.
- **Docs** — new `docs/shortcuts.md` (action × client matrix, where each chord lives in the code), `docs/config.md` (full TOML schema + per-OS path), `docs/homebrew.md` covers the desktop cask install + first-launch Gatekeeper workaround for the unsigned dmg.

### Fixed

- **Windows config path** — `outl-config::paths::config_dir()` now branches through `dirs::config_dir()` on Windows so the config lands under `%APPDATA%\outl\` (not `%USERPROFILE%\.config\outl\`, which is not a Windows convention).
- **`is_truthy` parity** — `outl_actions::page::is_truthy` no longer accepts `"pinned"` as a truthy literal; the set is now identical to `outl_md::index::is_truthy` (`true` / `yes` / `1` / `on`), so a hand-edited `.md` matches what the workspace index would also pick up.
- **fs_watcher Windows test** — `non_utf8_filename_is_ignored` is gated with `#[cfg(unix)]` (uses `OsStringExt::from_vec`), and `watched_root_label` tests now use `std::env::current_dir()` as a platform-portable absolute path anchor instead of the hardcoded `/tmp/ws` literal (not absolute on Windows).
- **Desktop outline scroll + narrow-window reflow** — body / `#root` now use `height: 100%` (was `min-height: 100vh`, which let the page grow with content and broke the height chain). `<main>` gained `min-w-0 min-h-0`; the AppShell grid template uses `minmax(0, 1fr)` instead of `1fr`. Same `min-width: auto` pitfall on both flex and grid axes; both unlocks pair.

### CI / Release

- **`desktop.yml`** — split into `check` (Linux, runs Clippy + Rust tests + Vitest + tsc + tauri bundle once) + `build` matrix (macOS arm64 + Windows x86_64 just compile + bundle). macOS x86_64 dropped from the PR matrix because the `macos-13` Intel runner pool is consistently depleted; release-time x86_64 binaries still ship via the universal dmg.
- **`release.yml`** — adds `build_desktop` (universal macOS dmg on `macos-latest`) and a single anchor in the bump-tap step so `Casks/outl-desktop-beta.rb` rides alongside `Formula/outl-beta.rb` on every push to main.

## [0.5.3] — 2026-06-02

**Unify backlinks, Insert-mode cross-block nav, anti-duplication policy.**

Two parallel backlinks pipelines (one on `outl-md::index`, one on `outl-actions`) had drifted on policy — self-references were dropped on the TUI panel but kept on mobile, and the user had to spot the divergence by comparing surfaces. 0.5.3 collapses them into one path through `outl_actions::backlinks_for_page`, deletes the cache on `outl-md::index`, and renames the related helpers so the call sites land on the shared API by default.

Insert mode also picks up the missing piece for vim/emacs muscle memory: `Up`/`Down` cross blocks (commit, move selection, re-enter Insert preserving the cursor column) the same way `Left`/`Right` already did.
Multi-line buffers (fenced code) absorb the move internally first.

### Added

- **`outl_core::Tree::properties_of(node)`** — iterator over every property currently set on a node, in one pass.
  Used by the outline DTO so each `OutlineNode` carries its own properties without scanning the workspace-wide map per block.
- **`outl_md::view::line_col_to_char(s, line, col)`** — inverse of the existing `char_to_line_col`.
  Vim-style column clamping (past EOL → end of line) and line clamping (past last → end of string).
  Lets `outl_tui::EditBuffer::move_up` / `move_down` wrap the same primitives the renderer (`block_to_rows`) already uses.
- **`outl_tui::EditBuffer::move_up` / `move_down` / `visual_column`** — three thin wrappers over `outl_md::view::char_to_line_col` + `line_col_to_char`.
  Cross-block Up/Down in Insert calls these first; only spills into the next block when the cursor was already on the buffer's first/last line.
- **`outl_actions::project_outline_node(workspace, node)`** — build a single `OutlineNode` (subtree + properties) from the workspace.
  Used by the backlinks builder so each backlink carries its source block as a self-contained outline.
- **`outl_actions::flatten_subtree_paths(node)`** — DFS-ordered paths inside an `OutlineNode` subtree.
  Moved here from `outl_md::outline_ops` so any client that consumes `Backlink::source_block` can navigate it.
- **`outl_actions::OutlineNode.properties`** — `(key, value)` pairs in alphabetical order.
  Workspace and disk paths both normalise to the same order so backlink panels and outline pages never disagree visually.
- **`outl_actions::PageMeta.icon`** — page-level `icon::` property surfaced on the meta.
  Clients pick their own fallback (mobile uses `📄`/`📅` by `kind`; TUI uses `📄`).

### Changed

- **Backlinks now route through `outl_actions::backlinks_for_page` only.** `outl_md::index::Backlink`, `WorkspaceIndex.backlinks()`, `refresh_backlinks_from_source`, `patch_backlink_text`, `flatten_backlink_subtree` were deleted.
  The `outl-md` index still owns page metadata and the block-level index; only the parallel backlinks cache went away.
- **`outl_actions::Backlink` is the rich struct.** Now carries `source_block: OutlineNode` (subtree + properties), `source_block_path: Vec<usize>`, `source_path: Option<PathBuf>` alongside `block_id`, `block_text` (TODO/DONE prefix stripped), `todo`, `source_page`.
  Mobile renders just `block_text` + `todo` today and ignores the rest; the TUI uses the full subtree to render its mini-outline in the backlinks panel.
- **`outl_actions::backlinks_for_page(workspace, root, meta)` / `backlinks_for_target(workspace, root, target)`** now take the workspace root so each backlink can carry its source `.md` path.
  CLI passes `&ctx.root`, TUI passes `&self.workspace_root`, mobile passes `storage_root`.
- **TUI cross-block Up/Down in Insert.** Commits the current buffer, moves the outline selection, re-enters Insert with the cursor on the preserved column.
  Guard: when `move_selection` would land `Focus` on the backlinks panel, the TUI stops in Normal mode instead of opening a different page mid-Insert.
  Backlink edits keep the older Esc → j/k → i workflow until cross-page commits get their own pass.
- **`App::backlinks_for_current` is cached.** Per-frame and per-keystroke render calls hit a `RefCell<Option<(slug, Vec)>>` cache; invalidated on `save`, `save_page_with`, `reload_workspace_from_disk`, and any view switch.
  Cuts the workspace scan from `O(blocks)` per call to one per slug change.
- **Self-references are kept in backlinks.** The "skip self-references as noise" heuristic on `outl_md::index` was dropped — a block on today's journal that mentions `[[2026-06-02]]` is exactly the "linked from" pin the user expects to see when revisiting that day.

### Refactored

- **`crates/outl-core/src/tree.rs` (854 lines) → `crates/outl-core/src/tree/{mod, cycle, op, apply}`** — `Tree::creates_cycle` in `cycle.rs`, `Tree::do_op` + `Tree::undo_op` in `op.rs`, `Tree::apply_op` in `apply.rs`.
  Struct and accessors stay in `mod.rs`.
  The 11 inline CRDT tests moved to `crates/outl-core/tests/tree_unit.rs`.
  **Algorithm semantics unchanged** — verified line-by-line against Kleppmann et al. 2022 and against the full invariant battery (convergence, cycle, cycle_chain, concurrent_edit_move, concurrent_delete_edit, late_op, idempotency, fractional_index, property_based, large_log: 32/32 green).
- **`crates/outl-tui/src/input.rs` (835 lines) → `crates/outl-tui/src/input/{mod, normal, insert, overlay, visual}`** — one handler per file, shared helpers (`cross_block_step`, `cursor_inside_open_fence`, `cross_block_nav_eligible`) stay in `mod.rs`.
- **`crates/outl-tui/src/actions/block.rs` (843 lines) → `crates/outl-tui/src/actions/block/{mod, insert, structural, backlink_edit, metadata}`** — Insert mode in `insert.rs`, create/indent/outdent/delete/move in `structural.rs`, cross-page backlink ops in `backlink_edit.rs`, properties + TODO toggle + pin in `metadata.rs`.
  TODO-prefix cycle helpers shared via `mod.rs`.
- **`crates/outl-tui/src/actions/lifecycle.rs` (669 lines) → `crates/outl-tui/src/actions/lifecycle/{mod, index_build, peer_sync, external, loading, persistence}`** — `App::new` and the shared `file_mtime` helper in `mod.rs`.
  Each submodule owns one concern.

No public API changed during the splits.
Clients (mobile, CLI, external consumers) need no update.

### Documentation

- **Anti-duplication policy** added to the root `CLAUDE.md` and echoed in every per-crate `CLAUDE.md`.
  Captures the lesson surfaced by the parallel `Backlink` structs and the near-miss with `line_start_and_column` (almost re-derived inside `EditBuffer` before the inverse `line_col_to_char` landed in `outl-md::view`).
  Rule: grep upstream first, prefer evolving the existing API over cloning the math.

### Internal

- `outl_md::Backlink`, `WorkspaceIndex.backlinks`, `refresh_backlinks_from_source`, `patch_backlink_text`, `flatten_backlink_subtree`, `outl_md::index::Backlink` removed.
- `outl_md::view` gained the `line_col_to_char` inverse.
- `outl_core::Tree.{nodes, properties, collapsed}` are now `pub(super)` so the split submodules can reach them.
  Public API unchanged.

## [0.5.1] — 2026-06-01

**Fix: multi-process writes against the same workspace.**

0.5.0 inherited an exclusive `flock` on `<root>/.outl/.lock` from the SQLite era.
The lock made sense when two writers on a single `log.db` would race, but JSONL stores one file per actor — the exclusive scope just blocked every legitimate co-tenant: TUI + MCP server, MCP server + `sink-outl` plugin, two CLI calls in flight.
Symptom: `INVALID_ARG: workspace ... is locked by another outl process` from the second opener, while the first ran fine and held the lock for its whole session.

0.5.1 splits coordination into two locks.
**Concurrent TUI + MCP server + CLI subprocess is the supported case** from here on.

### Added

- **`outl_core::WorkspaceLock` is now shared** (`LOCK_SH`).
  Every well-behaved `outl` process piles on.
  The lock still surfaces a hard filesystem error when `flock` itself fails, but never rejects a legitimate second opener.
- **`outl_core::ActorWriteLock`** — exclusive `flock` on `<root>/ops/.lock-<actor>`.
  Held by exactly one process per actor id at a time.
  This is the new write-coordination boundary.
- **`outl_core::resolve_write_actor(ops_dir, config_actor)`** — helper used by every workspace opener.
  Tries `config_actor` first; on `AlreadyHeld`, generates `ActorId::new()` and locks the ephemeral one instead.
  Returns the lock + actor id pair.
- **`WsCtx.ephemeral_actor: bool`** flag on the CLI/MCP context so `outl doctor` / `outl workspace info` can show when a process is writing under an ephemeral actor.

### Changed

- **`outl-cli::ws::open`** acquires the shared workspace lock plus a per-actor write lock through `resolve_write_actor`.
  On `outl` invocations that land while a server/TUI already holds the config actor, this process spins a fresh `ops-<ephemeral>.jsonl` and writes there.
  Readers merge every `ops-*.jsonl` in `ops/`, so peers see the full op log.
- **`outl-tui::open_workspace`** follows the same flow.
  The TUI used to refuse to launch when an MCP server was running against the same workspace; it now coexists.

### Why the ephemeral-actor fallback is safe

Every actor is independent at the CRDT layer (it's literally the mechanism multi-device sync relies on).
Two processes on the same device using two different actors merge the same way two devices would: the readers replay every `ops-<actor>.jsonl` in HLC order, the tree converges.
The only cost is `ops/` accumulating one jsonl per ephemeral lifetime — typically tiny files (a session's writes), and a future `outl gc` can consolidate them per device.

### Migration

None. 0.5.0 workspaces work as-is.
The next time you open a workspace with a second `outl` process, it will silently mint an ephemeral actor; the first process keeps writing under `config.toml[workspace].actor_id` as before.

## [0.5.0] — 2026-06-01

**Breaking: SQLite is gone.
JSONL is the only persistent storage.**

0.4.x kept two storage backends side by side — `SqliteStorage` for local-only workspaces and `JsonlStorage` for shared/synced ones.
The result was a class of "writes go through but disappear when you open the other client" bugs: any code path that opened a workspace via `outl-cli` got SQLite, while `outl-tui` and mobile (Tauri) followed `config.toml[workspace].storage` and got JSONL.
Same workspace, divergent op logs, silent loss.

0.5.0 collapses the surface: every client opens the workspace as `JsonlStorage` rooted at `<root>/ops/`.
There is no flag to choose, no `[workspace].storage` knob with two valid values, no SQLite fallback.
The `Storage` trait stays in place for future backends (ChronDB on the roadmap); the only impl that ships is JSONL plus the in-memory test double.

### Migration from 0.4.x

If your workspace was created with 0.4.x and you have data in `<root>/.outl/log.db`, the migration is a strict three-step sequence. 0.5.x cannot read SQLite and 0.4.1 is the last release that shipped `outl migrate-to-shared` (which this PR removed):

```bash
# 1. Pin 0.4.1 (last release with migrate-to-shared)
cargo install outl-cli --version 0.4.1 --locked

# 2. Run the one-shot migration (idempotent, leaves log.db intact)
outl migrate-to-shared <workspace>

# 3. Confirm ops/ops-<actor>.jsonl grew, then upgrade
cargo install outl-cli --version 0.5.1 --locked

# 4. Once you've verified peers see your data, delete log.db yourself
rm <workspace>/.outl/log.db <workspace>/.outl/log.db-shm <workspace>/.outl/log.db-wal
```

If you already had a mixed `log.db + ops/` workspace under 0.4.x, step 2 is still required — `migrate-to-shared` is idempotent (HLC dedup) and any ops that only ever made it into SQLite move over on this run.
After step 3, 0.5.x ignores `log.db` entirely.

### Removed

- **`SqliteStorage`** in `outl-core::storage`.
  Callers use `JsonlStorage` (persistent, per-actor JSONL) or `MemoryStorage` (the new in-memory test double, replaces `SqliteStorage::open_in_memory`).
- **`rusqlite` dependency.** Workspace `Cargo.toml` no longer pulls the SQLite C bundle.
  Faster builds, smaller binaries.
- **`outl migrate-to-shared`** subcommand.
  It only made sense while both backends coexisted; with only one backend the migration is a one-shot done on 0.4.1 before upgrading.
- **`config.toml[workspace].storage`** field is silently ignored now (kept readable so old configs don't error).
  Cleaning it up is fine but not required.

### Changed

- **`Paths` struct (`outl-cli/src/workspace_layout.rs`)** drops the `db: PathBuf` field, gains `ops: PathBuf` pointing at `<root>/ops/`.
  Every caller that touched `.outl/log.db` now targets the JSONL directory.
- **`outl init`** scaffolds `<root>/ops/` and opens `JsonlStorage` to materialize the per-actor `ops-<actor>.jsonl` file.
  The human output now reports `ops:` instead of `log:`.
- **`outl doctor`** drops the SQLite `PRAGMA integrity_check` finding and replaces it with a JSONL parse-and-load check (`JsonlStorage::open` surfaces every unreadable line via `tracing::warn!`, then the report carries the op count and the set of known node ids the sidecar cross-check needs).
- **`outl workspace info --json`** renames the `log_db` field to `ops_dir`.
  Stable-envelope shape otherwise unchanged.
- **`outl-tui::open_storage`** is now a one-liner.
  The config-driven match disappears; storage is always JSONL.
- **`Workspace::open_in_memory`** is unchanged in signature but uses the new `MemoryStorage` under the hood.
  No filesystem touch.

### Internal

- New `MemoryStorage` in `outl-core::storage::memory`.
  Pure `Vec<LogOp>` + snapshot slot, no I/O.
  Used by every test that previously called `SqliteStorage::open_in_memory()` and by `Workspace::open_in_memory`.

## [0.4.1] — 2026-06-01

Batch authoring for agents and scripts.
The 0.4.0 CLI / MCP surface covered every primitive write, but creating a structured page meant chaining N tool calls — one per block — which costs round-trips, turn budget on the agent, and time. 0.4.1 collapses that into the three composite shapes an agent or import pipeline actually wants: write a subtree, create a page with content, and stream a sequence of writes in one workspace session.

No storage or op-log format changes — every new tool is shimmed over the existing `outl-actions` primitives (`append_block`, `edit_text`, `set_property`).
Drop-in upgrade from 0.4.0.

### Added — composite write surface

- **`outl_block_append_tree` / `outl block append-tree`.** Append a root block plus its recursive children under a page or block in a single op-log session.
  Input shape: `{"text": "...", "children": [{"text": "...", "children": [...]}]}`.
  Response mirrors the input with `id` at every node so the caller can map specs back to freshly minted ids.
  CLI accepts the JSON inline (`--tree '{...}'`) or via stdin (`--tree -`).
- **`outl page create --content` / `outl_page_create` with `content`.** A new page lands with its full outline forest in one call instead of `page_create` + N × `block_append`.
  Accepts either a single root (`{text, children?}`) or a forest (`[{...}, {...}]`); the returned `content` array carries the block ids.
  Skipping the field keeps the historical empty-page behaviour.
- **`outl batch` / `outl_batch`.** Apply a list of writes sequentially in one workspace session.
  Supported `op` names: `page_create`, `page_update`, `page_delete`, `page_rename`, `block_append`, `block_append_tree`, `block_insert`, `block_update`, `block_move`, `block_delete`, `block_toggle_todo`, `daily_append`, `page_prop_set`.
  Each op's `args` mirror the matching standalone tool.
  **Stop-on-first-error semantics:** earlier ops stay in the op log (they're already CRDT ops; we don't roll them back), and the response carries `failed_at` / `failed_op` / `error` so the caller can recover or retry only the suffix that never ran.
  CLI exit code is `1` on partial failure.

### Added — `outl-actions::block`

- **`append_tree`, `append_forest`.** UI-agnostic primitives behind the new composite tools.
  `BlockTreeSpec` + `BlockTreeOutcome` are the shared DTOs (serde Serialize / Deserialize) so both client layers and future plugins can compose subtrees without re-deriving the recursion.

### Added — bench

- **`bench-cli-xlarge` workflow job.** Weekly + dispatch only.
  Generates a 10k-page batch payload via the new `xtask gen-10k` binary, applies it through `outl batch` end-to-end (subprocess + workspace lock + op log + sqlite + sidecar + md write), then runs `hyperfine` on `page list`, `search`, `query --tag`, `page get`, and `page render` against the populated workspace.
  Catches regressions in the surface that wraps the algorithm — the existing `bench-xlarge` job stays focused on the algorithm itself via criterion micro-benches.
- **`xtask` workspace member.** Internal task runner; today ships `gen-10k` (deterministic batch-payload generator) and is where any future codegen / fixture / bench helper lands.

### Docs

- `docs/cli.md` — new **Batch** section with the payload shape and failure semantics; `page create --content` and `block append-tree` documented inline next to the existing primitives.
- `docs/mcp.md` — multi-block authoring callout pointing at the three new composite tools.

## [0.4.0] — 2026-06-01

outl becomes scriptable.
A full machine-shaped CLI (page, block, daily, search, query, tag, prop, backlinks, export, workspace) lands with a stable JSON envelope and exit codes, and the same handlers are exposed over MCP via `outl mcp serve` (JSON-RPC over stdio) so Claude Desktop, Cursor, and any other agentic client can drive a workspace without parsing TUI output.
Business logic stays in `outl-actions`; the CLI and MCP are thin shims over the same code.

No storage or op-log format changes — drop-in upgrade from 0.3.x for data on disk.
**One breaking flag rename** for shell/cron users: `--path` is now `--workspace` everywhere.

### CLI (`outl-cli`) — new machine surface

- **Subcommands cover the full workspace API.** `outl page {list,get,create,rename,delete,prop}`, `outl block {get,edit,create,delete,move,toggle}`, `outl daily {today,get,range}`, `outl search`, `outl query`, `outl tag {list,page}`, `outl prop {list,page}`, `outl backlinks {page,block,embed}`, `outl export hugo`, `outl workspace {info,doctor}`.
  Every command writes a stable JSON envelope (`{ok, data, error, meta}`) to stdout and a typed exit code, so scripts and CI never have to scrape human output.
  `--human` keeps the friendly table format for interactive use.
- **One Workspace per process, index cached.** Each invocation opens the workspace once, reuses the in-memory index, and drops the per-call SQLite replay that older `outl serve`-style flows paid.
- **`--workspace` replaces `--path`.** The TUI, server, doctor, and every new subcommand now take `--workspace <dir>`.
  Existing scripts that pass `--path` must rename the flag (env var stays `OUTL_WORKSPACE`).
  The TUI's positional path argument is unchanged for direct double-clicks.
- **CLI integration suite** (`cli_machine.rs`) exercises page, block, search, and workspace commands against a real workspace so envelope shape and exit codes can't drift.

### MCP server (new: `outl mcp serve`)

- **JSON-RPC over stdio.** `outl mcp serve --workspace <dir>` speaks the MCP protocol with `initialize`, `tools/list`, `tools/call`, `resources/list`, `resources/read`, `prompts/list`, and `prompts/get`.
  Drop the binary into Claude Desktop's `claude_desktop_config.json` or Cursor's `mcp.json` and the agent can read journals, search, follow backlinks, edit blocks, and toggle TODOs against the same workspace your TUI/mobile is using.
- **Tools** mirror the CLI 1:1 (`outl_page_*`, `outl_block_*`, `outl_daily_*`, `outl_search`, `outl_query`, `outl_tag_*`, `outl_prop_*`, `outl_backlinks_*`, `outl_workspace_*`) so the LLM sees the same surface a human would script.
- **Resources** expose read-only views over `outl://daily/today`, `outl://page/<slug>`, `outl://search?q=…`, etc., for clients that prefer URI-addressed reads to tool calls.
- **Prompts** ship `summarize_day` and friends so the agent can pull a daily-note summary in one round-trip.
- **Per-session workspace + cached index.** The MCP server holds one `WsCtx` for the life of the session and routes every read through `ServerCtx::with_workspace`, which reuses that handle and invalidates the index after lazy journal materialisation in `outl://daily/today` and `summarize_day`.
  Earlier prototypes opened a fresh `WsCtx` per call and self-deadlocked on the workspace lock the session already owned — `resources/read` and `prompts/get` are now part of the same cached path as `tools/call`.
- **MCP smoke suite** (`mcp_smoke.rs`) walks `initialize` → `tools/list` → `tools/call` → `resources/read` in one session so the lock-reuse contract can't regress.

### Security / hardening

- **Slug validation at the boundary.** `outl-actions::is_valid_slug` rejects empties, `.`/`..` traversal, path separators, and control chars before any filesystem write, surfaced as a typed `ActionError::InvalidSlug` (`INVALID_ARG` in the CLI/MCP envelope).
  Hugo export adds a second `target_within` check against canonicalised paths so a legacy bad slug imported from disk still cannot escape `--out`.
- **Doctor split.** `workspace doctor` runs `collect_json` (full lock probe, used by `outl doctor` from the shell) and `collect_in_session_json` (probe off, used by the MCP tool which already owns the lock).
  Before this split, `outl_workspace_doctor` always warned about the workspace lock on perfectly healthy workspaces.
- **Quieter failures stop being silent.** Page delete/rename replace `let _ = remove_file(...)` with a `remove_or_warn` helper so a broken filesystem surfaces in logs instead of disappearing.
  Regression tests cover malicious slugs, doctor-clean inside an MCP session, and delete being idempotent when the `.md` is already gone.

### Docs

- New `docs/cli.md` and `docs/mcp.md` cover the machine surface and the MCP wiring for Claude Desktop / Cursor end to end (envelope shape, every subcommand, every tool, every resource).
- Getting-started, tutorial, sync, theming, TUI, and clients docs refreshed for the `--workspace` rename and the new subcommand names.

## [0.3.1] — 2026-05-31

Mobile UX polish + autocomplete fixes.
No protocol or storage changes — drop-in upgrade from 0.3.0.

### Mobile (`outl-mobile`)

- **Autocomplete (`[[…]]`) now actually fires on iOS.** The native ref suggester chip strip was orphaned — `createEffect` was being registered after an `await` inside `onMount`, which lost Solid's reactive owner.
  State was published once at boot and never updated as the user typed.
- **TODO/DONE prefix is visible (and editable) in Insert mode.** Tapping a TODO block used to show only the checkbox + body (`ship it`) with the `TODO ` prefix hidden, so erasing the prefix from the editor was impossible.
  Now the prefix appears in the textarea (`TODO ship it`) and the checkbox flips to a bullet while editing — toggling state via the text Just Works.
- **Cursor lands inside `[[ ]]` / `(( ))` reliably.** `el.value = …` resets the textarea caret in iOS WKWebView; combined with Solid's `value={draft()}` rebinding the caret could end up outside the pair.
  Replaced with `setRangeText` + double `parkCaret` (sync + microtask) so every toolbar insert, paste completion, and suggester pick parks the caret where the user expects it.
- **Backspace inside empty `[[]]` / `(())` collapses the pair.** No more mashing backspace four times to undo an aborted ref.
  Same rule on TUI and mobile.
- **Smart Punctuation is OFF.** `--` no longer becomes `–`, `...` no longer becomes `…`, quotes stay straight.
  Code snippets and CLI commands in journals survive intact.
- **Toast pattern for errors** (auto-dismiss + Retry button) in place of the persistent red `<p>` that sat in the middle of the outline forever.
  Failed saves now offer a one-tap retry without losing the draft.
- **`commitInFlight` lock + 8 s timeout** serializes concurrent block edits (typing → TODO toggle → blur) so the older save never overwrites the newer, and a stuck Tauri command can't freeze Insert mode indefinitely.
- **Progressive loading message** ("Loading…" → "Connecting to iCloud…" → "Still waiting on iCloud…") + spinner + a Retry button on terminal failure. iCloud cold-start no longer reads as "the app froze".
- **Connectivity-aware SyncDot** uses `navigator.onLine` + `online`/`offline` listeners to actually show the offline pip (was dead code before).
  `aria-label` instead of `title` so iOS WKWebView users get the status verbally.
- **Tap targets meet Apple HIG** (~30 × 30 hit area on the bullet/checkbox; bullet is now actually tappable).
  `[[ref]]` and `#tag` taps navigate instead of opening the editor.
- **Long-press TODO** uses a distinct success haptic when creating a new TODO vs. cycling an existing one.
- **`SwipeRow` × `SwipeNavigator` conflict resolved** — swipe-right on the left edge no longer races the per-row swipe-delete (each one captures only its own direction).
- **`PageSwitcher`** opens the first match on `Enter`, dismisses on `Escape`, and supports swipe-down on the handle to dismiss (without stealing scroll from the list).
- **Backlinks empty state** so the bidirectional-linking concept is discoverable on freshly-created pages.
- **Performance** in long outlines: `draft()` is now a lazy getter prop only read by the block being edited (was triggering a reactive effect in every BlockRow per keystroke).
  Auto-resize coalesced into a single `requestAnimationFrame`.

### Shared (`outl-actions`)

- `edit_text` writes its argument **verbatim** instead of preserving a leading `TODO `/`DONE ` prefix automatically.
  Callers that surface state separately (mobile checkbox) reattach the prefix themselves — required so erasing the prefix in the editor actually sticks.
  TUI path is unaffected (it always passes the raw block text through reconcile).

### TUI (`outl-tui`)

- Backspace inside an empty `[[]]` / `(())` now collapses both brackets in one keystroke (matches the mobile behaviour).

## [0.3.0] — 2026-05-30

Cross-device sync goes live.
A brand-new iOS app and the TUI share the same workspace over iCloud Drive, both driven by a shared `outl-actions` crate.
Block refs / embeds land in the markdown dialect.

### Mobile (`outl-mobile`) — new crate

- **Tauri 2 + SolidJS iOS client.** Bundle id `app.outl.mobile-app`, iCloud container `iCloud.app.outl.mobile-app`.
  Frontend is Solid + Tailwind; the Rust shell is intentionally thin (every workspace operation delegates to `outl-actions`).
- **iCloud Drive transport.** Workspace lives at `<ubiquity-container>/Documents/`.
  An ObjC bridge (`gen/apple/.../main.mm`) uses `NSMetadataQuery` to watch for peer changes and `NSFileCoordinator` + `startDownloadingUbiquitousItemAtURL` to force materialisation before reads — without those two steps the Rust side races the iCloud daemon and sees truncated op logs.
- **Per-device actor id** persisted under the app sandbox so each install writes to its own `ops-<actor>.jsonl`.
- iOS boot flash fixed; outl brand (palette + icon) applied across the app.

### Shared client (`outl-actions`) — new crate

- **Extracted every workspace mutation** (block edit, TODO toggle, indent / outdent, sibling create, delete, move, journal render, sync) out of `outl-tui` into a UI-agnostic crate.
  Functions take `&mut Workspace` + `&HlcGenerator` and route through `Workspace::apply` so the op log stays source of truth.
- TUI and mobile call the **same** functions for the same semantics — drift between clients is no longer possible by construction.
- `SyncEngine` interface owns the cross-device merge loop; iCloud is the v0 transport, iroh (phase 2) plugs in behind the same trait.

### Core (`outl-core`)

- **`JsonlStorage` op-log backend.** Single-file SQLite breaks under iCloud / Syncthing because the FS layer is last-write-wins per file.
  JSONL gives each actor its own `ops-<actor>.jsonl`, writes only to the local file, and merges every peer file on read.
- Folder layout is **`ops/`**, not `.ops/`. iCloud Documents skips dotted paths during cross-device sync — using a dot silently breaks multi-device workspaces.
  Same rule applied to the sidecar (`pages/<slug>.outl`, no leading dot).

### Markdown (`outl-md`)

- **`((blk-X))` inline refs and `!((blk-X))` embeds.** Stable `ref_handle` derived from the block's ULID tail (lazy 7+ char expansion on collision); sidecar bumped to v2.
  Embeds expand to the source root + children with a `↳` marker.
- Concurrent-safe writes over iCloud (atomic temp-file rename, no partial reads exposed to peers).
- `WorkspaceIndex` tracks block-ref backlinks alongside page-ref backlinks.

### TUI (`outl-tui`)

- Rebuilt as a **peer of shared workspaces** — same iCloud folder, same op log, same `outl-actions`.
  Edits on the laptop appear on the iPhone within seconds and vice versa.
- `((` autocomplete on block text, inline ref render, expanded embed view, Enter navigation to the source block, `/refer` and `/refer-embed` slash commands.
- `yr` chord copies the block's ref handle to the OS clipboard via arboard.
- outl brand (palette, icon, chrome) applied; mobile and TUI now look like the same product.

### CLI (`outl-cli`)

- **`outl migrate-to-shared` subcommand** rewrites a legacy SQLite workspace into the JSONL + sidecar layout consumed by both clients.
- `outl doctor` flags orphan `((blk-X))` and `!((blk-X))` handles.

### CI / release

- Release workflow rewritten as `prepare → tag → create_release (draft) → build matrix → publish_release`.
  Single `gh release create --draft` before the matrix and `gh release upload --clobber` per matrix leg, so paralleled jobs don't race each other on a repo with Immutable Releases turned on.
- macOS Intel binary now cross-compiles from `macos-latest` (ARM) instead of relying on the depleted `macos-13` runner pool.
- `outl-mobile` excluded from Linux CI jobs (Tauri iOS toolchain is macOS-only).

## [0.2.0] — 2026-05-26

Backlinks become a first-class part of the TUI: they live inline below the outline (no more side panel), render the referencing block with its children, and are fully editable in place.

### TUI (`outl-tui`)

- **Inline backlinks.** Replace the right-side panel with a section rendered below the outline, separated by a full-width `─` rule.
  Each source page shows up grouped under an icon + title header.
- **Full source block + children.** Backlinks render the referencing `OutlineNode` *with its subtree* (not a truncated snippet), so you see context without jumping to the source page.
- **Cursor navigation crosses the boundary.** `j`/`k` flow transparently between outline and backlinks.
  `app.focus: Focus::{Outline, Backlink{idx, sub_path}}` tracks where the cursor lives.
- **In-place edits land on the source `.md`.** `i`/`I`/`a`/`Esc`, `Ctrl+T` (TODO/DONE cycle), `o`/`O` (sibling create), `Tab`/`Shift+Tab` (indent/outdent), `dd` (delete), `K`/`J` (move up/down) — all work on a backlink the same way they work on the outline, persisting straight to the source page via `EditTarget::SourcePage`.
- **Optimistic index updates for snappy UX.** Edits patch the in-memory `WorkspaceIndex` immediately (next frame shows the new state), then save without scheduling a full workspace rebuild on the hot path.
- Cursor column preserved when entering Insert (`i` honors vim semantics; `I` still jumps home).
- Ghost cursor on the last outline block when focus had moved into the backlinks section is gone (`render_block` gates by `Focus::Outline`).
- `view.rs` split into `view/{inline, outline, overlays, backlinks}.rs` by responsibility — each file under 450 lines.

### Markdown (`outl-md`)

- `Backlink` carries the full `source_block: OutlineNode` and its `source_block_path` (DFS path in the source AST) instead of a flat index plus truncated snippet.
  Repeated refs to the same target inside one block collapse to a single backlink.
- `WorkspaceIndex::refresh_backlinks_from_source(path, &page)` — optimistic patch of every cached `source_block` for backlinks pointing at `path`.
  Used by the TUI's cross-page edit path.
- `WorkspaceIndex::patch_backlink_text(path, target_path, &new_text)` for text-only optimistic edits.

## [0.1.0] — 2026-05-25

First public release.
Single-device editor; sync transport is on the roadmap but the algorithm and op-log infrastructure are already in.

### Core (`outl-core`)

- Tree CRDT implementation following Kleppmann et al. 2022 (`do_op` / `undo_op` / `apply_op` / `creates_cycle`).
- HLC timestamps with actor tiebreak.
- Append-only op log with sqlite backend (`SqliteStorage`).
- `Storage` trait so alternative backends (e.g.
  ChronDB) can slot in without touching the CRDT.
- Workspace file lock via `fs2::flock` — two `outl` processes on the same workspace get a clean error, not a race.
- Property-based test of strong eventual consistency over 100+ randomised op permutations.

### Markdown / sidecar (`outl-md`)

- CommonMark parse + render with the outl dialect (`title::`, `icon::`, page/block properties, `[[refs]]`, `#tags`, `((block-id))`, fenced code blocks, multi-line block text).
- `.foo.outl` JSON sidecar holding the IDs the `.md` deliberately doesn't carry.
  **The `.md` stays clean** — no `id::`, no UUIDs.
- 3-level matching algorithm (`outl-md::matching`) reconstructs which block kept which ID after an external editor saves the file.
- Workspace index (`WorkspaceIndex`) — title, icon, slug, backlinks, tag namespace; powers the switcher, autocomplete and backlinks panel.
  Built once on boot, refreshed in a worker thread on save.
- Roundtrip property test: `parse(render(ast)) == ast` over randomly generated outlines including multi-line and fenced cases.

### Code-block execution (`outl-exec`)

- `Runtime` trait + `RuntimeRegistry`.
  Shipped runtimes (each behind a Cargo feature for opt-out distributions):
  - `lisp` — Steel (Scheme R5RS-ish in pure Rust).
  - `js` — Boa (ES2015+ in pure Rust).
  - `python` — RustPython (Python 3 subset).
  - `lua` — mlua 5.4 (vendored).
  - `rust` — `rustc → wasm32-wasip1 → wasmtime`.
    Compiled artefacts cached in `~/.cache/outl/runtimes/rust/<hash>.wasm`. ~20× faster on a re-run of the same snippet.
- WASM sandbox infrastructure (wasmtime engine + WASI ctx with no preopens / no env / no sockets, fuel-based instruction cap, epoch-interruption timeout, in-memory stdin/stdout/stderr).
- Idempotent result subblock — re-running the same code overwrites the existing `> **result:**` child instead of duplicating it.
- `source-hash::` stamped on each result child so the upcoming auto-run loop can short-circuit unchanged sources.

### TUI (`outl-tui`)

- Journal-first: opens on today's date.
- Vim-style modes (Normal / Insert / Visual) with chord support (`dd`, `gg`, `gx`, `yy`, `qq`-to-quit).
- Insert mode autocomplete for `[[refs]]`, `#tags`, and `/commands` (Notion-style slash menu).
- Slash command system + vim palette share one registry — every built-in command shows up in both.
  Built-ins: `prop-block`, `prop-page`, `search`, `run`, `theme`, `today`, `open`, `refresh`, `write`, `quit`, `help`.
  The registry is the plugin-extension point.
- `gx` runs the code block under the cursor through `outl-exec`.
- `auto-run::` property runs a block automatically on page open (cache-aware via SHA-256 of the source).
- `icon::` page property surfaces in every place the title shows (header, switcher, backlinks panel, search results, autocomplete, inline `[[refs]]`).
- Multi-line blocks via `Alt+Enter` / `Ctrl+J` / `Shift+Enter` (Shift+Enter only on terminals that speak the kitty keyboard protocol); plain `Enter` auto-detects an open code fence and inserts a soft newline inside it.
- Vertical scroll with `PgUp`/`PgDn`/`Ctrl+D`/`Ctrl+U`/`gg`/`G` and auto-scroll when the selection moves off-screen.
- Hot reload on external `.md` edits (polls mtime every 750ms; warns instead of clobbering when you're mid-Insert).
- Error modal overlay for multi-line failures (rustc compile errors, traps, missing toolchain), keeping the status line for short successes.
- Themes: 11 presets, switchable with `/theme <name>` at runtime.

### CLI (`outl-cli`)

- `outl` (no subcommand) opens the TUI in `$PWD`.
- `outl init <path>` scaffolds a workspace.
- `outl serve [--once]` reconciles `.md` files into the op log (one-shot or watch mode).
- `outl import logseq <src> <dst>` and `outl import roam <backup.json> <dst>` strip `id::` lines, slugify, seed sidecars.
- `outl doctor` and `outl reconcile` placeholders for the integrity and orphan-resolution flows.

### Tooling / DX

- Workspace MSRV: rustc 1.88.
- CI: `fmt` + `clippy -D warnings` + `cargo test --workspace --all-targets` on Linux and macOS.
- Bench CI: `small` / `medium` / `large` on every PR + push; `xlarge` (10k+ files) on weekly cron + manual dispatch.
- File-size guard hook (`.claude/hooks/file-size-guard.sh`) blocks Rust files past ~900 LOC unless the change is intentional — forces a refactor conversation before drift accumulates.
- Background workspace-index build: `App::new` paints the journal immediately and spawns a worker thread for the global index; backlinks/icons fill in within ~ms of boot.

### License

MIT.

[0.1.0]: https://github.com/avelino/outl/releases/tag/v0.1.0

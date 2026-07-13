# Paste

outl has two paste modes, and both do more than drop raw text into a block.
The goal is that whatever you copied — a Slack message, a Google Doc paragraph, an outline from another app, a chat reply — lands as a tidy outline in outl, not as a wall of unformatted text or a mess of stray characters.

- **Paste with formatting** converts the clipboard into outl markdown: rich formatting is kept, bullet structure becomes a real outline, and plain prose is split into one block per paragraph.
- **Paste without formatting** splices the raw clipboard text into the current block verbatim — no conversion, no splitting.

The chords are per-client; the full table lives in [Shortcuts](shortcuts.md).
In short: desktop `Cmd/Ctrl+V` (with) and `Cmd/Ctrl+Shift+V` (without); TUI `p` (with) and `Shift+P` (without); mobile is always with formatting.

## Paste with formatting

This is the default paste (`Cmd/Ctrl+V`, TUI `p`, every mobile paste).
It picks one of three routes, in order:

1. **Rich clipboard (`text/html`).**
   When you copy from an app that carries formatting — Slack, Google Docs, Notion, Gmail — the bold/italic/links/lists live in the clipboard's `text/html` flavour, while `text/plain` is stripped of them.
   outl reads the HTML and converts it to outl markdown, so the formatting survives:
   `**bold**`, `*italic*`, `[text](url)`, `- ` bullets, `~~strikethrough~~`, and inline code all come across.
   Custom emoji pasted as an image (Slack renders `:bus:` as `<img alt=":bus:">`) keep their `:shortcode:`.
   Editors that encode weight as inline CSS instead of `<b>` (Google Docs above all) are handled too: a `font-weight:700` span becomes `**bold**`, and the non-bold `<b>` wrapper Docs wraps the whole message in does not bold the entire block.

2. **Structured plain text.**
   With no richer HTML, if the clipboard is already an outline (lines starting with `- `) or has multiple paragraphs, it is routed through the conversion pipeline.
   An outline keeps its hierarchy; multi-paragraph prose (a pasted chat reply, an email) becomes one block per paragraph instead of a single wall-of-text block.
   Markdown copied from another outliner (Roam, Logseq, a GitHub task list) is normalised to the outl dialect on the way in.
   The full syntax-translation table is in [Markdown dialect → External paste](markdown-format.md#external-paste--outl-syntax).

3. **Trivial text.**
   A single word, a URL, one line — spliced into the block in place, no round-trip, so a routine paste stays instant.

The routing decision is one shared function, `choosePasteRoute`, so the desktop and mobile clients can never disagree about what a given clipboard should do.

## Paste without formatting

`Cmd/Ctrl+Shift+V` (desktop) and `Shift+P` (TUI) paste the raw clipboard text with **no** conversion:
the text is spliced into the current block as-is, and outline-looking or multi-paragraph content is **not** split.
Use it when you want the literal characters — a code snippet, a block of text whose line breaks matter, markdown you want to keep as source rather than render.

Mobile has no without-formatting chord; every mobile paste is with formatting.

## Pasting inside a code block

When the block you are editing is a fenced code block (a `` ``` `` block), **every** paste is literal — the same as paste-without-formatting — regardless of the chord you use.
A multi-line or outline-shaped clipboard is spliced in verbatim, with its line breaks intact, instead of being converted into sibling blocks (which would tear the fence apart and strand the closing `` ``` `` on its own line).
This holds on both GUI clients (desktop and mobile), because the guard lives next to the shared `choosePasteRoute` decision.

## Under the hood

The behaviour is shared across clients so it stays identical everywhere:

- **`choosePasteRoute(html, plain)`** (`@outl/shared/paste`) — the rich / structured / native decision, shared by desktop and mobile.
- **`htmlToOutlMarkdown(html)`** (`@outl/shared/paste`) — the `text/html` → outl markdown conversion, built on [Turndown](https://github.com/mixmark-io/turndown) tuned for the outl dialect.
- **`outl_actions::paste_markdown`** — with-formatting: normalises external syntax, detects outline shape, splits paragraphs, and grafts the result as blocks through the op log.
- **`outl_actions::paste_plain`** — without-formatting: raw text as one block, no normalisation or splitting.

The TUI reads the OS clipboard directly (`arboard`) and runs the same `outl_actions` pipeline; it has no `text/html` flavour to convert, so rich-clipboard conversion is a GUI-only capability.
On the desktop, paste-without-formatting (`Cmd/Ctrl+Shift+V`) reads the clipboard through the Tauri clipboard-manager plugin (backend `arboard`), not `navigator.clipboard.readText()`.
The macOS webview gates that web API behind a native "Paste" permission button when it is called outside a real paste gesture, so the plugin read is what makes the chord work.
See the [Shared primitives catalog](shared-primitives.md) for where each piece lives.

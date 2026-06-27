# ASCII Box

An example outl plugin: wrap the body of a ` ```box ` fence in a drawn ASCII box.

It's the smallest demo of a **text** content-transformer:

- **`content-transformer:text`** — register a function for a code-fence language; the host runs it with the fence body and renders the descriptor you return.
- Because the descriptor is `kind: "text"`, it renders on **every** client — desktop, mobile, **and the TUI/CLI** (no webview required).

The box itself is drawn in `src/index.ts` (`boxify`) — **by the plugin author, not the host.**
The engine knows nothing about "boxes"; it only transports the string you produced.
Want rounded corners or a double border? Swap the glyphs.

## Example

````text
```box
hello
world!
```
````

renders into:

```text
┌────────┐
│ hello  │
│ world! │
└────────┘
```

The box is as wide as the longest line; every line is padded to match, with a one-space gutter inside the border.

## Build + install

```bash
bun install                  # from the repo root, once
cd examples/box && bun run build   # bundles src/index.ts → index.js (IIFE)

outl -w <workspace> plugin install ./examples/box --yes
```

Then put a ` ```box ` fence in any page and open it on any client.

## How transformers work

A transformer is a **pure function**: `fn(body)` returns `{ kind, content }` or `null` (to decline).
`kind: "text"` content is plain text rendered everywhere.
`kind: "rich"` content is HTML run in a sandboxed iframe (GUI clients only — see the `bars` example).
The host calls your function only for the `lang` you declared under `contributes.transformers`, so clients can skip fences no plugin handles.

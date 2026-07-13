# Plugins

outl ships a JavaScript plugin system so the ceiling of what you can do isn't pinned to what the maintainers ship.
That's the lesson from Roam and Logseq: the moment a tool can't be extended, every unmet need becomes a fork request that never lands.

The bet is simple: **a plugin is written once and runs on every client that renders or edits** — TUI, desktop, mobile, and the CLI.
Plugins are plain JavaScript, the largest ecosystem and the lowest barrier to entry.

## Pick your path

By what you're here to do:

- **Use plugins** — you're on the right page; keep reading ([install](#installing), [permissions](#permissions), [where they live](#where-plugins-live)).
- **Write one** — start with the hands-on [Plugin tutorial](plugin-tutorial.md), keep the [Plugin API](plugin-api.md) open while you work.
- **Copy a working starting point** — grab one of the [example plugins](plugin-examples.md), one per capability.
- **Understand the internals** — read the [Plugin architecture](plugin-architecture.md) (the Boa engine, the describe→apply model, the safety model).

## How it works, in one paragraph

A plugin never touches your `.md` files or the CRDT directly.
Every mutation it performs flows `JS → host API → outl-actions → Workspace::apply → op log`, and every op a plugin produces is stamped with `actor = "plugin:<id>@<device>"`.
The op log stays the source of truth, your markdown stays 100% clean, and the log doubles as an audit trail of exactly what each plugin did.

## Installing

Plugins install per workspace from a **local directory** or a **`github:` source**:

```sh
outl plugin install ./outl-todo-archiver               # local dir
outl plugin install github:avelino/outl/examples/todo-archiver   # github
outl plugin install github:user/repo#v1.2.0            # pin a tag
```

A `github:owner/repo[/subdir][#tag]` source is cloned at an **immutable semver tag** — the newest published tag when none is pinned, never a mutable branch like `main`.
Either way, install validates `plugin.json`, copies the installed shape into the workspace, computes the bundle hash, shows you the permissions the plugin requests, and asks for approval before anything is written.
The resolved version, source, approved permissions, and bundle hash are recorded in the lockfile (see [Where plugins live](#where-plugins-live)).

> **Scaffolding a new plugin:** `outl plugin init <name>` writes a buildable starter project (manifest + `package.json` + `tsconfig` + `src/index.ts`); run `bun install && bun run build` inside it for an installable bundle.
> Today's CLI surface is `init`, `list`, `install`, `run`, `enable`, `disable`, `remove`.
> `outl plugin update` and `.outlpkg` packaging are still roadmap.

### Running a command

A plugin command runs headless through the CLI:

```sh
outl plugin run <id> <command>
```

In the interactive clients the same command shows up on each client's command surface — see below.

### Command discovery per client

A `slash-command` plugin contributes a command that the user can run; where it surfaces depends on the client:

| Client | How you run a plugin command |
|---|---|
| TUI | Press `/` (Normal mode) → the slash menu lists built-ins **and** plugin commands, keyed by the command **id** (`/stats`); type to filter, `Enter` to run. |
| Desktop | Two ways: the `⧉` button (bottom-left chrome) opens the plugin palette, **and** typing `/` at the start of a block opens an inline slash menu (Notion-style) — `/stats` ranks the `stats` command on top, `Enter` runs it. |
| Mobile | The plugin sheet (header) lists the commands; tap to run. |
| CLI | `outl plugin run <id> <command>` (headless). |

The command **id** is the canonical name across every surface (the CLI's `outl plugin run … stats`, the TUI/desktop `/stats`), so a plugin author picks one id and it reads the same everywhere.
The inline desktop `/` menu only triggers **block-initial** (a mid-text slash is a path or URL, never a command).

### Enabling, disabling, listing, removing

```sh
outl plugin list
outl plugin enable <id>
outl plugin disable <id>
outl plugin remove <id>     # aliases: uninstall, rm
```

Enabled/disabled state is the `enabled` flag in the lockfile (`installed.json`).
Disabling keeps the plugin installed but stops it from loading.
`remove` is the opposite of `install`: it deletes the plugin's directory under `.outl/plugins/<id>/` and drops its lockfile entry.

## Permissions

A plugin declares the permissions it needs in its manifest.
You approve them **once, on install**, and the approved set is frozen in the lockfile.
Every host call is gated against that set — a plugin that didn't get `write-page` cannot write, no matter what its code tries.

| Permission | Grants |
|---|---|
| `read-page` | Read page and block content. |
| `write-page` | Create or edit page and block content. |
| `read-op-log` | Observe ops as they're applied (the `onOp` hook). |
| `submit-op` | Submit mutations to the op log. |
| `storage:local` | Per-plugin local key/value storage (this device only — does **not** sync). |
| `network:<domain>` | Network access scoped to one domain. |

Network is always scoped to a domain.
`network:api.openai.com` (exact) and `network:*.openai.com` (leading-label wildcard) are valid; a bare `network:*` is **rejected** — a plugin can never request the whole internet.

> **storage:local does not converge.**
> Per-plugin KV storage is local to the device on the day-zero release, to keep the op log from inflating.
> If a plugin needs state that syncs across your devices, that state has to be modeled as an op (not supported yet) — it won't silently appear on your other machines.

## Where plugins live

Installed plugins sit inside the workspace, next to your notes:

```
<workspace>/.outl/plugins/<id>/
├── plugin.json
├── index.js            # the single bundled file
├── index.js.map        # optional, for better errors
├── config.schema.json
└── README.md
```

Only the **build output** lands here — no `node_modules`, no source tree.
The rule is hard: a plugin survives deleting `node_modules`.

### The lockfile

Each workspace keeps an `installed.json` lockfile recording, per plugin:

- `version` and `source` (the install source ref — a local path today, an immutable `github:…#vX.Y.Z` tag once that source lands)
- `bundleHash` — sha256 of `index.js`, **revalidated on every load**
- `permissionsApproved` — the frozen approved set
- `installedBy` — the device that installed it
- `config` — your settings, stored outside the bundle so they survive a reinstall
- `enabled`

The `bundleHash` is the integrity check.
If `index.js` ever differs from the recorded hash — an out-of-band edit through iCloud or Finder, a half-finished sync — the load is **blocked** rather than silently running modified code.
The installed version never changes underneath you.

`installedBy` records the device that installed the plugin, so a synced workspace can tell "approved here" from "approved on another device".
(The cross-device re-confirm prompt that uses it is roadmap; today the hash check is the active gate.)

## Dev mode

While building a plugin, drop it in `.outl/plugins/_dev/<name>/` inside the workspace instead of installing it:

- **No bundle-hash check** — it loads straight from the directory, so you can rebuild and reload without reinstalling.
- **Permissions are implicit** — every permission the manifest declares is granted, no approval prompt.
- **Never recorded in the lockfile** and excluded from sync, so dev iterations don't leak to your other devices.

This is for authoring only — a `_dev` plugin runs with a relaxed sandbox.
A hot-reload-on-save watcher and an in-client "sandbox relaxed" banner are roadmap niceties; the load behavior above is what ships today.
See [Plugin API → Anatomy](plugin-api.md#anatomy-of-a-plugin) for the full dev layout.

## Distribution

### The registry

Discovery is a static index — `registry.json` — versioned in the [`outlmd/registry`](https://github.com/outlmd/registry) repo and served at **`https://plugins.outl.app/registry.json`** (Netlify, static, with CORS so any client can fetch it).
No server, no infrastructure.
It lists each plugin's id, name, `github:` repo, published versions, capabilities, permissions, and description — what powers the discovery list and search.
In-client discovery (`outl plugin search` + a browse/install screen in the desktop & mobile apps) reads this index.
A hosted registry (`registry.outl.app`) with full-text search and install counts is deferred until volume justifies it.

### Publishing your plugin (registering it)

Listing your plugin in the registry is what makes it show up in the in-app marketplace (the desktop/mobile browse-and-install screen) and in `outl plugin search`.
The registry stores **only metadata** — your code stays in your repo; the bundle host (`plugins.outl.app`) re-downloads it from there at build time.

Four steps:

1. **Host the built plugin in a public GitHub repo.**
   The repo's default branch must contain the **installed shape** at the path you'll point the registry at: `plugin.json`, the bundled `index.js` (run `bun run build`), and `config.schema.json` if you have one.
   Either at the repo root, or in a subdirectory (a monorepo of plugins works — point at `owner/repo/path/to/plugin`).

2. **Open a PR against [`outlmd/registry`](https://github.com/outlmd/registry)** adding one entry to `registry.json`:

   ```jsonc
   {
     "id": "dev.you.my-plugin",            // MUST equal your plugin.json `id`
     "name": "My Plugin",
     "description": "One sentence on what it does.",
     "author": "your-handle",
     "repository": "github:you/my-plugin", // or github:you/repo/subdir
     "category": "productivity",
     "keywords": ["..."],
     "capabilities": ["slash-command"],     // mirror plugin.json
     "permissions": ["read-page"],          // mirror plugin.json — users see the ask
     "latest": "1.0.0",
     "versions": ["1.0.0"]
   }
   ```

   The `id`, `capabilities`, and `permissions` **must** match your `plugin.json` (CI validates the entry against the schema; the install re-validates the manifest anyway).

3. **Merge → the Netlify build re-fetches your bundle** from the repo and serves it at `https://plugins.outl.app/p/<id>/`.

4. Your plugin now appears in **`outl plugin search`** and the **in-app marketplace**, installable with one tap.

> **Official vs. unofficial.**
> The in-app marketplace only installs plugins **listed in the registry** (so a tap-to-install is always something a human reviewed in the PR).
> A plugin that isn't listed yet — yours mid-development, a private one, a fork — installs via the CLI instead: `outl plugin install github:you/repo` or `outl plugin install ./dir`.
> Full reference for publishing (the schema, the build, pinning a tag) lives in the registry repo's [README](https://github.com/outlmd/registry#adding-a-plugin).

### `.outlpkg` (roadmap)

A `.outlpkg` will be the installed shape of a plugin — manifest, bundle, and assets, **no source** — packed as tar+gzip, named `<id>-<version>.outlpkg`, with its own extension (not `.zip`) so the OS can associate it with outl.
It is **not implemented yet**: today, install is from a local directory (and, once wired, a `github:` source).

## Capabilities per client

A capability is something a plugin plugs into; each client implements a subset.
The loader **intersects** the two — a capability your current client can't honor loads partially with a warning, never a crash.
The plugin still runs for everything else.

| Capability | TUI | Desktop | Mobile | CLI |
|---|:---:|:---:|:---:|:---:|
| `op-hook` | ✅ | ✅ | ✅ | ✅ |
| `slash-command` | ✅ | ✅ | ✅ | ✅ |
| `config-schema` (read) | ◑ | ◑ | ◑ | ◑ |
| `keybinding` | ✅ | ✅ | — | — |
| `toolbar-button` | ✅ (slash menu) | ✅ | ✅ | — |
| `content-transformer:text` | ✅ | ✅ | ✅ | — |
| `content-transformer:rich` | — | ✅ | ✅ | — |
| `sync-transport` | core only — client polling is roadmap | | | |

✅ implemented · ◑ partial · 🔜 planned (post day-zero) · — not applicable to this client.

`op-hook` and `slash-command` run identically on every client (the CLI exposes commands through `outl plugin run`).
**`config-schema` is partial**: a plugin can *read* its config with `ctx.config.get()` (the value comes from the lockfile), but there is no config-editing form UI yet, and the schema isn't enforced on the stored value.
**`keybinding` is live on the TUI and the desktop.**
A `contributes.keybindings` chord fires the bound command — on the TUI from Normal mode (single- and two-chord sequences), on the desktop wherever a native binding doesn't claim it first.
Use a free chord like `Ctrl+G` or a two-chord sequence such as `Ctrl+G A`.
Mobile has no keyboard, so it doesn't apply there.
**`toolbar-button` is live**: desktop and mobile render a button in the chrome for the plugin's command, and the TUI surfaces that command in its slash menu (a terminal has no chrome bar).
**`content-transformer` is live** today: `ctx.content.register(lang, fn)` renders a fenced block — `:text` on every read surface (inline in the TUI), `:rich` as HTML in a sandboxed iframe on the GUIs (the TUI drops it).
A plugin that wants to be a query engine registers a transformer for the `query` fence; plugins can also call `outl.query({ … })` from JS code blocks to get structured results (see [Query code blocks → Plugin SDK API](query.md#plugin-sdk-api-outlquery)).
**`sync-transport` is core-ready**: `ctx.sync.register({ push, pull })` works and convergence is tested, but no client polls the transport on a timer yet — that wiring is roadmap.
The CLI is headless, so anything visual or chord-driven (`keybinding`, `toolbar-button`, `content-transformer:*`) doesn't apply to it.

## Permissions reference

| Wire string | Permission |
|---|---|
| `read-page` | Read page/block content |
| `write-page` | Create/edit page/block content |
| `read-op-log` | Observe applied ops |
| `submit-op` | Submit ops to the log |
| `storage:local` | Per-plugin local KV (no sync) |
| `network:<domain>` | Network to one domain (`network:*` rejected) |

## See also

- [Plugin tutorial](plugin-tutorial.md) — build a plugin step by step.
- [Plugin API](plugin-api.md) — the authoring reference: manifest, host API, `definePlugin`, versioning.
- [Plugin architecture](plugin-architecture.md) — how the runtime works under the hood.
- [`plugin-v1.json`](schemas/plugin-v1.json) — JSON Schema for `plugin.json`.
- [CLI](cli.md) — the `outl plugin` subcommands.

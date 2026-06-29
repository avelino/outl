//! `outl plugin init <name>` — scaffold a new plugin project.
//!
//! Writes the dev-time shape (manifest + `package.json` + `tsconfig` +
//! `src/index.ts` + README) so an author runs `bun install && bun run build`
//! and has an installable bundle. Mirrors the layout of `examples/*`.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

/// Scaffold a plugin named `name` (used for the directory + display name).
/// `id` is the reverse-DNS plugin id (defaults to `com.example.<slug>`).
/// `dir` overrides the output directory (defaults to `./<slug>`).
pub fn run(name: &str, id: Option<&str>, dir: Option<&Path>) -> Result<()> {
    let slug = slugify(name);
    if slug.is_empty() {
        bail!("`{name}` has no usable letters/digits for a directory name");
    }
    let id = id
        .map(str::to_string)
        .unwrap_or_else(|| format!("com.example.{slug}"));
    let out = dir
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(&slug));

    if out.exists()
        && out
            .read_dir()
            .map(|mut d| d.next().is_some())
            .unwrap_or(false)
    {
        bail!("`{}` already exists and isn't empty", out.display());
    }
    std::fs::create_dir_all(out.join("src"))
        .with_context(|| format!("creating {}", out.display()))?;

    write(&out.join("plugin.json"), &plugin_json(&id, name))?;
    write(&out.join("package.json"), &package_json(&slug))?;
    write(&out.join("tsconfig.json"), TSCONFIG)?;
    write(&out.join("src/index.ts"), &index_ts(name))?;
    write(&out.join("README.md"), &readme(name, &id))?;
    write(&out.join(".gitignore"), GITIGNORE)?;

    println!("Scaffolded {name} ({id}) in {}/", out.display());
    println!("Next:");
    println!("  cd {}", out.display());
    println!("  bun install && bun run build       # produces index.js");
    println!(
        "  outl plugin install ./{}            # try it in a workspace",
        out.display()
    );
    Ok(())
}

fn write(path: &Path, contents: &str) -> Result<()> {
    std::fs::write(path, contents).with_context(|| format!("writing {}", path.display()))
}

/// Lowercase, keep `[a-z0-9-]`, collapse the rest to single `-`.
fn slugify(name: &str) -> String {
    let mut s = String::new();
    let mut prev_dash = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            s.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash && !s.is_empty() {
            s.push('-');
            prev_dash = true;
        }
    }
    s.trim_matches('-').to_string()
}

fn plugin_json(id: &str, name: &str) -> String {
    format!(
        r#"{{
  "$schema": "https://outl.app/schemas/plugin-v1.json",
  "id": "{id}",
  "name": "{name}",
  "version": "0.1.0",
  "api": "^1.0",
  "engines": {{
    "outl": ">=0.7.0"
  }},
  "main": "index.js",
  "description": "A new outl plugin.",
  "author": "you <you@example.com>",
  "license": "MIT",
  "category": "misc",
  "capabilities": [
    "slash-command"
  ],
  "permissions": [],
  "contributes": {{
    "commands": [
      {{
        "id": "hello",
        "title": "{name}: say hello"
      }}
    ]
  }}
}}
"#
    )
}

fn package_json(slug: &str) -> String {
    format!(
        r#"{{
  "name": "outl-{slug}",
  "version": "0.1.0",
  "description": "An outl plugin.",
  "license": "MIT",
  "private": true,
  "type": "module",
  "scripts": {{
    "build": "esbuild src/index.ts --bundle --format=iife --platform=neutral --target=es2022 --outfile=index.js",
    "typecheck": "tsc --noEmit"
  }},
  "dependencies": {{
    "@outl/plugin-sdk": "^1.0.0"
  }},
  "devDependencies": {{
    "esbuild": "^0.24.0",
    "typescript": "^6.0.3"
  }}
}}
"#
    )
}

const TSCONFIG: &str = r#"{
  "compilerOptions": {
    "target": "ES2022",
    "module": "ESNext",
    "lib": ["ES2022", "DOM"],
    "moduleResolution": "bundler",
    "skipLibCheck": true,
    "isolatedModules": true,
    "esModuleInterop": true,
    "forceConsistentCasingInFileNames": true,
    "noEmit": true,

    "strict": true,
    "noUnusedLocals": true,
    "noUnusedParameters": true,
    "noFallthroughCasesInSwitch": true
  },
  "include": ["src"]
}
"#;

fn index_ts(name: &str) -> String {
    format!(
        r#"/**
 * {name} — an outl plugin.
 *
 * Starter scaffold: a single `hello` slash command that toasts a message.
 * Grow it by adding capabilities to `plugin.json` and wiring them here —
 * see https://github.com/avelino/outl/blob/main/docs/plugin-api.md
 */

import {{ definePlugin, type PluginContext }} from "@outl/plugin-sdk";

export default definePlugin({{
  activate(ctx: PluginContext) {{
    ctx.commands.register("hello", () => {{
      ctx.ui.notify("👋 Hello from {name}!");
    }});
  }},
}});
"#
    )
}

fn readme(name: &str, id: &str) -> String {
    format!(
        r#"# {name}

An [outl](https://github.com/avelino/outl) plugin (`{id}`).

## Build

```sh
bun install        # or npm install
bun run build      # bundles src/index.ts → index.js
```

## Try it

```sh
outl plugin install ./{slug}      # installs into the current workspace
outl plugin run {id} hello
```

In the TUI / desktop, type `/hello` in a block.

## Develop

Edit `src/index.ts`, rebuild, and reinstall (or drop the folder in
`<workspace>/.outl/plugins/_dev/{slug}/` to skip the hash check while iterating).
The full host API — `ctx.blocks`, `ctx.ops.onOp`, `ctx.content.register`,
`ctx.net.fetch`, `ctx.storage`, … — is documented at
<https://github.com/avelino/outl/blob/main/docs/plugin-api.md>.
"#,
        slug = slugify(name),
    )
}

const GITIGNORE: &str = "node_modules/\n*.log\n";

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn slugify_handles_spaces_and_symbols() {
        assert_eq!(slugify("My Cool Plugin!"), "my-cool-plugin");
        assert_eq!(slugify("  Trailing  "), "trailing");
        assert_eq!(slugify("a/b/c"), "a-b-c");
    }

    #[test]
    fn scaffolds_a_buildable_shape() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("my-plugin");
        run("My Plugin", None, Some(&dir)).unwrap();

        for f in [
            "plugin.json",
            "package.json",
            "tsconfig.json",
            "src/index.ts",
            "README.md",
            ".gitignore",
        ] {
            assert!(dir.join(f).is_file(), "missing {f}");
        }

        // The manifest must parse + validate through the real loader.
        let bytes = std::fs::read(dir.join("plugin.json")).unwrap();
        let manifest = outl_plugins::PluginManifest::parse(&bytes).expect("valid manifest");
        assert_eq!(manifest.id, "com.example.my-plugin");
        assert_eq!(manifest.name, "My Plugin");
    }

    #[test]
    fn custom_id_is_honored() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("p");
        run("Thing", Some("dev.avelino.thing"), Some(&dir)).unwrap();
        let bytes = std::fs::read(dir.join("plugin.json")).unwrap();
        let manifest = outl_plugins::PluginManifest::parse(&bytes).unwrap();
        assert_eq!(manifest.id, "dev.avelino.thing");
    }

    #[test]
    fn refuses_a_nonempty_dir() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("x"), "y").unwrap();
        assert!(run("P", None, Some(tmp.path())).is_err());
    }
}

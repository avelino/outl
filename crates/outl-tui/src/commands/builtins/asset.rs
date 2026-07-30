//! `/upload` — attach a file and insert its link at the cursor.
//!
//! Wiring only: the copy-into-workspace + content-hash + link markdown
//! all live in `outl_actions::import_asset`. This command just resolves a
//! source path (a native OS file dialog on macOS / Windows, a typed path
//! on Linux) and splices the returned `[name](assets/…)` link into the
//! live Insert buffer — the same insertion path the drag-drop paste and
//! the date inserters use.

use std::path::PathBuf;

use anyhow::Result;
use outl_actions::import_asset;

use super::super::SlashCommand;
use crate::state::{App, Mode};

/// `/upload` — pick a file and insert its asset link at the cursor.
pub struct UploadCommand;

impl SlashCommand for UploadCommand {
    fn name(&self) -> &'static str {
        "upload"
    }

    fn description(&self) -> &'static str {
        "attach a file and insert its link at the cursor"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["attach"]
    }

    /// macOS / Windows open the OS file dialog with no argument. Linux
    /// has no bundled picker (`rfd` would pull GTK into the CI build), so
    /// there the command takes a path and the slash menu hands off to the
    /// `:` palette pre-filled `upload `.
    fn needs_args(&self) -> bool {
        cfg!(target_os = "linux")
    }

    /// Writes the link straight into the live buffer, so the slash
    /// dispatcher must keep the Insert alive (no commit before running).
    fn inserts_inline(&self) -> bool {
        true
    }

    fn execute(&self, app: &mut App, args: &str) -> Result<bool> {
        // An explicit path argument wins (the Linux prompt, or
        // `:upload <path>` from the palette on any platform); otherwise
        // open the native picker.
        let source = if args.trim().is_empty() {
            pick_file()
        } else {
            Some(PathBuf::from(args.trim()))
        };
        let Some(source) = source else {
            app.status = "upload cancelled".into();
            return Ok(false);
        };
        if !source.is_file() {
            app.status = format!("not a file: {}", source.display());
            return Ok(false);
        }

        let max_bytes = outl_config::load().assets.max_bytes;
        match import_asset(&app.workspace_root, &source, max_bytes) {
            Ok(asset) => insert_or_warn(app, &asset.markdown),
            Err(e) => app.status = format!("upload failed: {e}"),
        }
        Ok(false)
    }
}

/// Insert `markdown` at the cursor when in Insert mode; otherwise warn —
/// like the date inserters, `/upload` only makes sense while typing.
fn insert_or_warn(app: &mut App, markdown: &str) {
    if let Mode::Insert { buffer, .. } = &mut app.mode {
        buffer.insert_str(markdown);
        app.status = format!("attached {markdown}");
    } else {
        app.status = format!("/upload only works in Insert mode (would insert {markdown})");
    }
}

/// Open the OS "open file" dialog. macOS / Windows only; on Linux this is
/// always `None` (the command is arg-taking there) so no GTK dependency
/// reaches the workspace CI build.
#[cfg(not(target_os = "linux"))]
fn pick_file() -> Option<PathBuf> {
    rfd::FileDialog::new().pick_file()
}

/// Linux stand-in: there is no bundled dialog, so a no-arg `/upload`
/// yields nothing and the command's `needs_args()` steers the user to the
/// typed-path prompt instead.
#[cfg(target_os = "linux")]
fn pick_file() -> Option<PathBuf> {
    None
}

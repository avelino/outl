//! Resolving a `outl plugin install` source to a local directory.
//!
//! Two source shapes today:
//! - a **local path** (a directory holding `plugin.json` + bundle), and
//! - a **`github:` source**, cloned at an immutable semver tag.
//!
//! The git side shells out to `git` (no libgit dependency); the network +
//! process work lives here so `plugin.rs` stays focused on the install flow.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use semver::Version;
use tempfile::TempDir;

/// A parsed `github:` source: `github:owner/repo[/subdir][#tag]`.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct GithubSource {
    pub owner: String,
    pub repo: String,
    /// Path to the plugin inside the repo, if it isn't at the root.
    pub subdir: Option<String>,
    /// Pinned tag (`v1.2.0`), or `None` to resolve the newest semver tag.
    pub tag: Option<String>,
}

/// A resolved install source: a local directory plus the `source` string
/// recorded in the lockfile. `_guard` keeps a clone's temp dir alive for
/// the lifetime of the value (dropped → directory removed).
pub(crate) struct ResolvedSource {
    pub dir: PathBuf,
    pub source_ref: String,
    _guard: Option<TempDir>,
}

impl ResolvedSource {
    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

/// Resolve any `source` string to a local directory containing a
/// `plugin.json`. `github:` clones at a tag; anything else is treated as a
/// local path.
pub(crate) fn resolve(source: &str) -> Result<ResolvedSource> {
    if source.starts_with("github:") {
        let gh = parse_github(source)?;
        return clone_github(&gh);
    }
    let dir = PathBuf::from(source);
    if !dir.is_dir() {
        bail!("`{source}` is not a directory");
    }
    Ok(ResolvedSource {
        source_ref: format!("local:{source}"),
        dir,
        _guard: None,
    })
}

/// Parse `github:owner/repo[/subdir…][#tag]`.
pub(crate) fn parse_github(source: &str) -> Result<GithubSource> {
    let rest = source
        .strip_prefix("github:")
        .context("not a github: source")?;
    let (locator, tag) = match rest.split_once('#') {
        Some((l, t)) if !t.is_empty() => (l, Some(t.to_string())),
        Some((l, _)) => (l, None),
        None => (rest, None),
    };
    let mut parts = locator.splitn(3, '/');
    let owner = parts
        .next()
        .filter(|s| !s.is_empty())
        .context("github source is missing an owner: expected github:owner/repo")?
        .to_string();
    let repo = parts
        .next()
        .filter(|s| !s.is_empty())
        .context("github source is missing a repo: expected github:owner/repo")?
        .to_string();
    let subdir = parts
        .next()
        .map(|s| s.trim_matches('/').to_string())
        .filter(|s| !s.is_empty());
    Ok(GithubSource {
        owner,
        repo,
        subdir,
        tag,
    })
}

/// Clone the repo into a temp dir, check out an immutable tag (the pinned
/// one or the newest semver tag), and return the plugin directory.
///
/// The install never tracks a mutable branch — a tag is required, matching
/// the lockfile's integrity guarantee (the bundle hash is frozen at a
/// specific published version).
fn clone_github(gh: &GithubSource) -> Result<ResolvedSource> {
    let tmp = TempDir::new().context("creating a temp dir for the clone")?;
    let repo_dir = tmp.path().join("repo");
    let url = format!("https://github.com/{}/{}.git", gh.owner, gh.repo);

    git(&[
        "clone",
        "--quiet",
        "--no-checkout",
        &url,
        repo_dir.to_str().context("temp path is not UTF-8")?,
    ])
    .with_context(|| format!("cloning {url}"))?;

    let tag = match &gh.tag {
        Some(t) => t.clone(),
        None => newest_semver_tag(&repo_dir).with_context(|| {
            format!(
                "no semver tag found in {}/{} — a plugin must publish a tag (install never tracks a mutable branch)",
                gh.owner, gh.repo
            )
        })?,
    };
    git_in(&repo_dir, &["checkout", "--quiet", &tag])
        .with_context(|| format!("checking out tag `{tag}`"))?;

    let plugin_dir = match &gh.subdir {
        Some(s) => repo_dir.join(s),
        None => repo_dir.clone(),
    };
    if !plugin_dir.join("plugin.json").is_file() {
        let where_ = gh.subdir.as_deref().unwrap_or("the repo root");
        bail!(
            "no plugin.json at {where_} in {}/{}@{tag}",
            gh.owner,
            gh.repo
        );
    }

    let mut source_ref = format!("github:{}/{}", gh.owner, gh.repo);
    if let Some(s) = &gh.subdir {
        source_ref.push('/');
        source_ref.push_str(s);
    }
    source_ref.push('#');
    source_ref.push_str(&tag);

    Ok(ResolvedSource {
        dir: plugin_dir,
        source_ref,
        _guard: Some(tmp),
    })
}

/// The newest tag that parses as semver (a leading `v` is tolerated).
fn newest_semver_tag(repo_dir: &Path) -> Result<String> {
    let out = capture(repo_dir, &["tag", "--list"])?;
    let best = out
        .lines()
        .filter_map(|raw| {
            let raw = raw.trim();
            Version::parse(raw.strip_prefix('v').unwrap_or(raw))
                .ok()
                .map(|v| (v, raw.to_string()))
        })
        .max_by(|a, b| a.0.cmp(&b.0));
    match best {
        Some((_, tag)) => Ok(tag),
        None => bail!("no semver-shaped tags"),
    }
}

/// Run `git <args>`, failing on a non-zero exit.
fn git(args: &[&str]) -> Result<()> {
    let status = Command::new("git")
        .args(args)
        .status()
        .context("running `git` (is it installed and on PATH?)")?;
    if !status.success() {
        bail!("git {} failed", args.join(" "));
    }
    Ok(())
}

/// Run `git -C <dir> <args>`, failing on a non-zero exit.
fn git_in(dir: &Path, args: &[&str]) -> Result<()> {
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .status()
        .context("running `git`")?;
    if !status.success() {
        bail!("git -C … {} failed", args.join(" "));
    }
    Ok(())
}

/// Run `git -C <dir> <args>` and return stdout.
fn capture(dir: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .context("running `git`")?;
    if !out.status.success() {
        bail!("git -C … {} failed", args.join(" "));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_owner_repo() {
        assert_eq!(
            parse_github("github:outlmd/todo-archiver").unwrap(),
            GithubSource {
                owner: "outlmd".into(),
                repo: "todo-archiver".into(),
                subdir: None,
                tag: None,
            }
        );
    }

    #[test]
    fn parses_pinned_tag() {
        let gh = parse_github("github:outlmd/examples#v1.2.0").unwrap();
        assert_eq!(gh.tag.as_deref(), Some("v1.2.0"));
        assert_eq!(gh.subdir, None);
    }

    #[test]
    fn parses_subdir_and_tag() {
        let gh = parse_github("github:outlmd/examples/confetti#v0.3.1").unwrap();
        assert_eq!(gh.owner, "outlmd");
        assert_eq!(gh.repo, "examples");
        assert_eq!(gh.subdir.as_deref(), Some("confetti"));
        assert_eq!(gh.tag.as_deref(), Some("v0.3.1"));
    }

    #[test]
    fn parses_nested_subdir() {
        let gh = parse_github("github:outlmd/examples/plugins/confetti").unwrap();
        assert_eq!(gh.subdir.as_deref(), Some("plugins/confetti"));
    }

    #[test]
    fn rejects_missing_repo() {
        assert!(parse_github("github:outlmd").is_err());
        assert!(parse_github("github:").is_err());
    }

    #[test]
    fn local_path_resolves_to_itself() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("plugin.json"), "{}").unwrap();
        let resolved = resolve(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(resolved.dir(), dir.path());
        assert!(resolved.source_ref.starts_with("local:"));
    }
}

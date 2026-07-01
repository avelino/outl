//! Pages and their journal variant.
//!
//! A **page** is a direct child of [`NodeId::root`] whose
//! `page-slug` property identifies it. The page's text is its
//! user-facing title; the page's tree children are its blocks.
//!
//! Journals are pages with `page-kind = "journal"` and a
//! date-shaped slug (`YYYY-MM-DD`). The rest of the schema is
//! identical to regular pages.
//!
//! Why properties instead of a separate node kind: keeping pages as
//! ordinary nodes means the tree CRDT still owns all the move /
//! delete / re-parent semantics for free, and the wire format stays
//! one Op log. Pages are just nodes with a marker property.

use chrono::{Duration, NaiveDate};
use outl_core::hlc::HlcGenerator;
use outl_core::id::NodeId;
use outl_core::op::{LogOp, Op};
use outl_core::property::PropValue;
use outl_core::workspace::Workspace;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::block::create_with_explicit_id;
use crate::error::ActionError;
use crate::tree::{children_of, position_for_new_last_child};

/// Property key marking a node as a page root and recording its slug.
pub const SLUG_KEY: &str = "page-slug";
/// Property key recording whether a page is a regular page or a
/// journal.
pub const KIND_KEY: &str = "page-kind";
// `TYPE_KEY` / `PERSON_TYPE` / `search_persons` / `ensure_person_by_name`
// live in the sibling `person` module so this file stays focused on the
// page-model primitives (slug, kind, title, journal). The constants and
// `search_persons` are re-exported at the crate root via `lib.rs` for
// back-compat with callers that imported them from `outl_actions::page`.

/// Whether a page is a regular named page or a date-keyed journal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PageKind {
    /// Regular named page.
    Page,
    /// Date-keyed journal page.
    Journal,
}

impl PageKind {
    /// Wire value used in the stored property.
    pub fn as_str(self) -> &'static str {
        match self {
            PageKind::Page => "page",
            PageKind::Journal => "journal",
        }
    }

    /// Parse from the stored property value, defaulting to
    /// [`PageKind::Page`] when the value is missing or unknown.
    pub fn parse(value: Option<&PropValue>) -> Self {
        match value {
            Some(PropValue::Text(s)) if s == "journal" => PageKind::Journal,
            _ => PageKind::Page,
        }
    }
}

/// UI-friendly summary of a page.
#[derive(Debug, Clone, Serialize)]
pub struct PageMeta {
    /// Stringified [`NodeId`] of the page root.
    pub id: String,
    /// Stable slug identifying the page (filename-safe).
    pub slug: String,
    /// Human-readable title. Resolved most-specific-first: the
    /// `title::` page property (where ingest / reconcile park the name),
    /// then the page node's own text (set by in-app creation), then the
    /// slug as a last resort. See `page_meta`.
    pub title: String,
    /// `page` or `journal`.
    pub kind: PageKind,
    /// Optional emoji / icon string the user set on the page (via the
    /// `icon::` page property). `None` when unset — clients pick their
    /// own fallback (mobile uses the page kind to decide between 📄
    /// and 📅; TUI uses `📄` for everything by default).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// `pinned:: true` page-level property. Surfaces that ship a
    /// sidebar (TUI, desktop) list pinned pages prominently so the
    /// user can pin their canonical workspace entry points (people,
    /// "inbox", "weekly review", …). Default `false` and serialised
    /// only when `true` so the wire stays small.
    ///
    /// Mirrors the `WorkspaceIndex::PageEntry.pinned` field already
    /// owned by `outl-md` — but lifted onto `PageMeta` so every
    /// client that lists pages (`list_all_pages`) sees the flag
    /// without needing to also consult the index.
    #[serde(default, skip_serializing_if = "is_false")]
    pub pinned: bool,
    /// `type::` page-level property (lowercased+trimmed). `None` when
    /// unset. Drives the `@` mention autocomplete: clients filter the
    /// list to `Some("person")` candidates without re-reading the
    /// workspace index. Mirrors `WorkspaceIndex::PageEntry.page_type`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_type: Option<String>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// Look up a page by slug. Returns the page root [`NodeId`] when found.
pub fn find_by_slug(workspace: &Workspace, slug: &str) -> Option<NodeId> {
    children_of(workspace, NodeId::root())
        .into_iter()
        .find_map(|(id, _)| {
            let prop = workspace.tree().property(id, SLUG_KEY)?;
            match prop {
                PropValue::Text(s) if s == slug => Some(id),
                _ => None,
            }
        })
}

/// List every page in the workspace, sorted by slug.
pub fn list_all(workspace: &Workspace) -> Vec<PageMeta> {
    let mut pages: Vec<PageMeta> = children_of(workspace, NodeId::root())
        .into_iter()
        .filter_map(|(id, _)| page_meta(workspace, id))
        .collect();
    pages.sort_by(|a, b| a.slug.cmp(&b.slug));
    pages
}

/// Read page metadata from a node. Returns `None` when the node is
/// not a page (no `page-slug` property).
pub fn page_meta(workspace: &Workspace, id: NodeId) -> Option<PageMeta> {
    let slug = match workspace.tree().property(id, SLUG_KEY)? {
        PropValue::Text(s) => s.clone(),
        _ => return None,
    };
    let kind = PageKind::parse(workspace.tree().property(id, KIND_KEY));
    // Title resolution, most-specific first:
    //   1. the `title::` page property — where ingest / reconcile parks
    //      the human-readable name (`diff_to_ops_with_page_props` emits
    //      it as `Op::SetProp` key `"title"`, and the page root's text
    //      stays empty for disk-sourced pages);
    //   2. the page node's own text — set when a page is created
    //      in-app via `open_or_create_by_name` (the typed name);
    //   3. the slug — last resort so a title is never empty.
    // Reading `block_text` alone (the old behaviour) made every
    // ingested page surface its slug in pickers / autocomplete.
    let title = match workspace.tree().property(id, "title") {
        Some(PropValue::Text(s)) if !s.trim().is_empty() => s.trim().to_string(),
        _ => workspace
            .block_text(id)
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| slug.clone()),
    };
    // `icon::` is a free-form page property. It survives as a
    // `PropValue::Text` after `reconcile_md` applies the page's
    // properties; we surface only the textual variant so the wire
    // format stays a plain string. Other PropValue shapes (PageRef /
    // Tag / List) are not legal for an icon and would silently
    // mismatch the renderer, so we treat them as absent.
    let icon = match workspace.tree().property(id, "icon") {
        Some(PropValue::Text(s)) if !s.trim().is_empty() => Some(s.trim().to_string()),
        _ => None,
    };
    // `pinned::` is a page-level boolean property. We accept the
    // **exact** set of truthy literals `outl-md::index::is_truthy`
    // does (`true`, `yes`, `1`, `on`) so a hand-edited `.md`
    // matches what the workspace index would also pick up. Don't
    // add new tokens here without updating outl-md in the same
    // commit — drift breaks the "every list of pages agrees on
    // pinned" invariant.
    let pinned = match workspace.tree().property(id, "pinned") {
        Some(PropValue::Text(s)) => is_truthy(s),
        _ => false,
    };
    // `type::` is a free-form page-level property (`person`, `project`,
    // …). We surface it lowercased + trimmed so callers can compare
    // against `PERSON_TYPE` without re-normalising. Same shape as the
    // workspace index — see `outl_md::index::PageEntry.page_type`.
    let page_type = match workspace.tree().property(id, crate::person::TYPE_KEY) {
        Some(PropValue::Text(s)) => {
            let normalised = s.trim().to_lowercase();
            if normalised.is_empty() {
                None
            } else {
                Some(normalised)
            }
        }
        _ => None,
    };
    Some(PageMeta {
        id: id.to_string(),
        slug,
        title,
        kind,
        icon,
        pinned,
        page_type,
    })
}

fn is_truthy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "true" | "yes" | "1" | "on"
    )
}

/// Deterministic [`NodeId`] derived from a page slug.
///
/// Two devices independently creating the same slug — even offline,
/// before iCloud / Syncthing / shared FS has had a chance to
/// reconcile — end up with the **same** [`NodeId`]. Without this, each
/// peer would mint a fresh random ULID and we'd have two divergent
/// page subtrees with no way to merge them (the CRDT only merges
/// concurrent edits to the *same* node).
///
/// We use `sha256("outl-page:" + slug)[..16]` as the ULID's 128-bit
/// payload. The constant prefix avoids accidental collisions with any
/// other content-derived id scheme that might enter the workspace
/// later. Output is deterministic and stable across releases.
pub fn page_id_from_slug(slug: &str) -> NodeId {
    let mut h = Sha256::new();
    h.update(b"outl-page:");
    h.update(slug.as_bytes());
    let digest = h.finalize();
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    NodeId(ulid::Ulid::from_bytes(bytes))
}

/// Whether `slug` is safe to use as a single path component for the
/// `.md` / `.outl` projection.
///
/// The slug is joined into `pages/<slug>.md` (or `journals/...`) and
/// shows up in `[[refs]]`, in `block-ref` handles, and in the export
/// pipelines. Anything that would escape its directory (`..`, `/`,
/// `\`) or smuggle control characters (`\0`, newline) is rejected
/// here so a single check covers every downstream surface. Leading /
/// trailing whitespace is also rejected because it silently breaks
/// filename equality across iCloud / git / external editors.
pub fn is_valid_slug(slug: &str) -> bool {
    if slug.is_empty() || slug.len() > 255 {
        return false;
    }
    if slug != slug.trim() {
        return false;
    }
    if slug == "." || slug == ".." {
        return false;
    }
    for ch in slug.chars() {
        match ch {
            '/' | '\\' | '\0' | '\n' | '\r' | '\t' => return false,
            c if c.is_control() => return false,
            _ => {}
        }
    }
    // No `..` segment hidden in something like `foo/../bar` even though
    // we already reject `/`. Belt and suspenders for any caller that
    // routes the slug through a path join.
    !slug.split(['/', '\\']).any(|c| c == "..")
}

/// Open the page for `slug`, creating it if it doesn't exist yet.
///
/// The page's [`NodeId`] is derived deterministically from the slug
/// (see [`page_id_from_slug`]), so a peer that locally creates the
/// same slug ends up writing to the same node. iCloud sync then
/// merges the two creators' edits into one page instead of leaving
/// two competing copies.
///
/// Rejects slugs that fail [`is_valid_slug`] — the slug ends up
/// joined into a filesystem path, so anything that could escape its
/// directory (`..`, `/`, `\`, control chars) stays out of the
/// workspace entirely.
pub fn open_or_create(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    slug: &str,
    title: &str,
    kind: PageKind,
) -> Result<NodeId, ActionError> {
    if !is_valid_slug(slug) {
        return Err(ActionError::InvalidSlug(slug.to_string()));
    }
    if let Some(id) = find_by_slug(workspace, slug) {
        return Ok(id);
    }
    let node_id = page_id_from_slug(slug);
    let position = position_for_new_last_child(workspace, NodeId::root());
    let node = create_with_explicit_id(
        workspace,
        hlc,
        node_id,
        NodeId::root(),
        position,
        Some(title),
    )?;
    set_prop(
        workspace,
        hlc,
        node,
        SLUG_KEY,
        PropValue::Text(slug.to_string()),
    )?;
    set_prop(
        workspace,
        hlc,
        node,
        KIND_KEY,
        PropValue::Text(kind.as_str().to_string()),
    )?;
    Ok(node)
}

/// Open (or create) a page identified by a user-typed **name** —
/// the kind of string that arrives from a `[[ref]]`, a `#tag`, or a
/// page-picker text field.
///
/// The name flows through [`outl_md::slug::slugify`] before reaching
/// [`open_or_create`], so anything filesystem-hostile (`/`, `\`,
/// spaces, accented letters, control chars) is normalised into a
/// safe single path component. The **original** `name` is kept as the
/// page's `title` so the user-facing rendering stays verbatim
/// (`[[avelino/outl]]` displays as `avelino/outl` even though the
/// disk slug is `avelino-outl`).
///
/// Use this whenever the caller has a human-typed string and wants
/// "open it or create it on the fly" semantics — the [`open_or_create`]
/// path requires a pre-validated slug and rejects raw user input. The
/// TUI's `Enter`-on-ref handler and the mobile's `open_page_by_slug`
/// command are both fed by this function so the two clients can't
/// drift on what counts as a valid ref target.
pub fn open_or_create_by_name(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    name: &str,
    kind: PageKind,
) -> Result<NodeId, ActionError> {
    let slug = outl_md::slug::slugify(name);
    open_or_create(workspace, hlc, &slug, name, kind)
}

/// Open (or create) whatever page a user-typed **ref target** points
/// at — `[[avelino/outl]]`, `[[2026-06-04]]`, `[[São Paulo]]`,
/// `[[Q4 plan]]`, a `#tag` body, a picker query. Routes through one
/// of:
///
/// 1. **Date-shaped target → journal**. `date_from_slug` is the
///    semantic validator (not the regex-shape one the mobile
///    frontend used to use), so `2026-13-01` and `2026-02-30` fall
///    through instead of erroring out.
/// 2. **Literal slug match → existing page**. Covers picker-style
///    callers that already passed a clean slug.
/// 3. **Slugified match → existing page**. So `[[avelino/outl]]`
///    finds the page stored as `avelino-outl` even when the user
///    typed the ref before the page existed.
/// 4. **Case-insensitive title match → existing page**. Last
///    existing-page chance before we create.
/// 5. **Create a fresh page via [`open_or_create_by_name`]** with
///    the typed string as the title. Always succeeds for any ref a
///    user could plausibly type — the surface should never bubble
///    `invalid …` back to a tap.
///
/// One canonical decision tree, used by every client, so the
/// "Journal vs Page" discrimination cannot drift between frontend
/// regex and backend parser the way it did in the
/// `[[2026-13-01]] → invalid date slug` toast bug.
pub fn open_or_create_by_ref(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    target: &str,
) -> Result<NodeId, ActionError> {
    if let Some(date) = date_from_slug(target) {
        return open_journal(workspace, hlc, date);
    }
    // **Mention sugar — must run BEFORE the generic slug/title match.**
    //
    // `slugify("@avelino")` strips the `@`, returning `"avelino"`. If we
    // ran the generic `find_by_slug(slugify(target))` branch first, a
    // pre-existing `pages/avelino.md` (created before this feature, or
    // by an external editor without `type:: person`) would resolve via
    // the generic path and return early — never reaching the arm that
    // marks it as a person. The autocomplete popup would then never
    // surface that page on the next `@` keystroke, even though the
    // user just resolved a mention against it. Keep this arm first.
    if let Some(rest) = target.strip_prefix('@') {
        if !rest.is_empty() {
            return crate::person::ensure_person_by_name(workspace, hlc, rest);
        }
        // `[[@]]` — empty name. Fall through to the generic path,
        // which will create an `untitled` page via slugifier. Not
        // great UX but consistent with `[[]]`; refusing here would
        // require an extra error path the callers don't model.
    }
    if let Some(id) = find_by_slug(workspace, target) {
        return Ok(id);
    }
    let normalised = outl_md::slug::slugify(target);
    if normalised != target {
        if let Some(id) = find_by_slug(workspace, &normalised) {
            return Ok(id);
        }
    }
    let lower = target.to_lowercase();
    if let Some(existing) = list_all(workspace)
        .into_iter()
        .find(|p| p.title.to_lowercase() == lower)
    {
        use std::str::FromStr;
        if let Ok(id) = ulid::Ulid::from_str(&existing.id) {
            return Ok(NodeId(id));
        }
    }
    open_or_create_by_name(workspace, hlc, target, PageKind::Page)
}

fn set_prop(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    node: NodeId,
    key: &str,
    value: PropValue,
) -> Result<(), ActionError> {
    set_property(workspace, hlc, node, key, Some(value))
}

/// Set (or clear, with `value = None`) a property on `node`.
///
/// One-liner over `Op::SetProp` exposed so every client can write
/// properties without reaching into `outl-core::Op` directly. The
/// `set_prop` private alias above keeps backward-compat with the
/// internal call sites in this module.
pub fn set_property(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    node: NodeId,
    key: &str,
    value: Option<PropValue>,
) -> Result<(), ActionError> {
    let ts = hlc.next();
    workspace.apply(LogOp {
        ts,
        actor: ts.actor,
        op: Op::SetProp {
            node,
            key: key.to_string(),
            value,
            old_value: None,
        },
    })?;
    Ok(())
}

/// Read a property as a plain `String`. Returns `None` when the
/// property is unset or is a structured value the caller would have
/// to unwrap (`List`); in those cases, read `workspace.tree().property`
/// directly.
///
/// Convenience for the export / display surfaces that just want the
/// raw text of `icon::`, `title::`, single-tag `tags::`, etc.
pub fn read_text_prop(workspace: &Workspace, node: NodeId, key: &str) -> Option<String> {
    match workspace.tree().property(node, key)? {
        PropValue::Text(s) | PropValue::PageRef(s) | PropValue::Tag(s) => Some(s.clone()),
        PropValue::List(items) => {
            let joined = items
                .iter()
                .filter_map(|v| match v {
                    PropValue::Text(s) | PropValue::PageRef(s) | PropValue::Tag(s) => {
                        Some(s.clone())
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(" ");
            if joined.is_empty() {
                None
            } else {
                Some(joined)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Journal helpers
// ---------------------------------------------------------------------------

/// Slug for `date` using the canonical `YYYY-MM-DD` shape.
pub fn journal_slug(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

/// Display title for `date`. We use ISO `YYYY-MM-DD` because it
/// matches the slug 1:1, sorts naturally, and stays compact in
/// constrained UI (mobile header).
pub fn journal_title(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

/// Today's date in the user's configured timezone (falling back to the
/// OS local timezone). Delegates to [`crate::clock`] so the journal's
/// "today" honours `[calendar] timezone` — see issue #107.
pub fn today() -> NaiveDate {
    crate::clock::today()
}

/// Open today's journal page, creating it if needed.
pub fn open_today(workspace: &mut Workspace, hlc: &HlcGenerator) -> Result<NodeId, ActionError> {
    open_journal(workspace, hlc, today())
}

/// Open the journal page for `date`, creating it if needed.
pub fn open_journal(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    date: NaiveDate,
) -> Result<NodeId, ActionError> {
    open_or_create(
        workspace,
        hlc,
        &journal_slug(date),
        &journal_title(date),
        PageKind::Journal,
    )
}

/// Parse a `YYYY-MM-DD` slug back into a `NaiveDate`.
pub fn date_from_slug(slug: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(slug, "%Y-%m-%d").ok()
}

/// Previous calendar day relative to `date`.
pub fn previous_journal_date(date: NaiveDate) -> NaiveDate {
    date - Duration::days(1)
}

/// Next calendar day relative to `date`.
pub fn next_journal_date(date: NaiveDate) -> NaiveDate {
    date + Duration::days(1)
}

/// Sweep any legacy children of [`NodeId::root`] that aren't pages
/// (have no `page-slug` property) under today's journal. Used once
/// during the migration from pre-page-model workspaces.
pub fn migrate_legacy_into_today(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
) -> Result<usize, ActionError> {
    let today_id = open_today(workspace, hlc)?;
    let stragglers: Vec<NodeId> = children_of(workspace, NodeId::root())
        .into_iter()
        .filter_map(|(id, _)| {
            if id == today_id {
                return None;
            }
            if workspace.tree().property(id, SLUG_KEY).is_some() {
                return None;
            }
            Some(id)
        })
        .collect();

    let mut moved = 0usize;
    for node in stragglers {
        crate::tree::position_for_new_last_child(workspace, today_id);
        let pos = crate::tree::position_for_new_last_child(workspace, today_id);
        let old_parent = workspace
            .tree()
            .parent(node)
            .ok_or_else(|| ActionError::NotInTree(node.to_string()))?;
        let old_position = workspace
            .tree()
            .position(node)
            .cloned()
            .ok_or_else(|| ActionError::MissingPosition(node.to_string()))?;
        let ts = hlc.next();
        workspace.apply(LogOp {
            ts,
            actor: ts.actor,
            op: Op::Move {
                node,
                new_parent: today_id,
                position: pos,
                old_parent,
                old_position,
            },
        })?;
        moved += 1;
    }
    Ok(moved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::append_block;
    use outl_core::id::ActorId;

    fn ws() -> (Workspace, HlcGenerator) {
        let actor = ActorId::new();
        (
            Workspace::open_in_memory(actor).unwrap(),
            HlcGenerator::new(actor),
        )
    }

    #[test]
    fn create_and_find_by_slug() {
        let (mut w, hlc) = ws();
        let id = open_or_create(&mut w, &hlc, "ideas", "Ideas", PageKind::Page).unwrap();
        assert_eq!(find_by_slug(&w, "ideas"), Some(id));
        let pages = list_all(&w);
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].slug, "ideas");
        assert_eq!(pages[0].title, "Ideas");
        assert_eq!(pages[0].kind, PageKind::Page);
    }

    #[test]
    fn open_or_create_by_ref_routes_valid_date_to_journal() {
        // Use a fixed past date so the intent ("any valid YYYY-MM-DD
        // → journal") is obvious. Today's date would compile and pass
        // but suggests a system-clock coupling the function does not
        // actually have.
        let (mut w, hlc) = ws();
        let id = open_or_create_by_ref(&mut w, &hlc, "2020-01-01").unwrap();
        let meta = page_meta(&w, id).unwrap();
        assert_eq!(meta.kind, PageKind::Journal);
        assert_eq!(meta.slug, "2020-01-01");
    }

    #[test]
    fn open_or_create_by_ref_falls_through_invalid_date_to_page() {
        // Regression: the mobile frontend used to gate this with the
        // shape regex `/^\d{4}-\d{2}-\d{2}$/`. `2026-13-01` (month 13)
        // and `2026-02-30` (day 30 in Feb) passed the shape, then
        // `parse_date` rejected them and the user saw an
        // `invalid date slug` toast. The shared helper has the only
        // discrimination path, so an invalid date becomes a regular
        // page just like any other ref.
        let (mut w, hlc) = ws();
        let bogus = "2026-13-01";
        let id = open_or_create_by_ref(&mut w, &hlc, bogus).unwrap();
        let meta = page_meta(&w, id).unwrap();
        assert_eq!(meta.kind, PageKind::Page);
        assert_eq!(meta.title, bogus);
        assert_eq!(meta.slug, "2026-13-01");
    }

    #[test]
    fn open_or_create_by_ref_resolves_existing_via_slugified_form() {
        let (mut w, hlc) = ws();
        // Create with the human name first.
        let created = open_or_create_by_name(&mut w, &hlc, "avelino/outl", PageKind::Page).unwrap();
        // Subsequent tap of `[[avelino/outl]]` finds the same node
        // (not a fresh ULID under a different slug).
        let resolved = open_or_create_by_ref(&mut w, &hlc, "avelino/outl").unwrap();
        assert_eq!(created, resolved);
    }

    #[test]
    fn open_or_create_by_ref_matches_title_case_insensitively() {
        let (mut w, hlc) = ws();
        let created = open_or_create_by_name(&mut w, &hlc, "Ideas", PageKind::Page).unwrap();
        let resolved = open_or_create_by_ref(&mut w, &hlc, "ideas").unwrap();
        assert_eq!(created, resolved);
    }

    #[test]
    fn open_or_create_by_name_slugifies_filesystem_hostile_input() {
        // Regression: clicking `[[avelino/outl]]` on mobile used to
        // bubble the `/` straight into `is_valid_slug` and surface
        // `invalid page slug` as a toast. The helper must normalise
        // the disk slug while keeping the typed name as the title so
        // the ref renders verbatim everywhere.
        let (mut w, hlc) = ws();
        let id = open_or_create_by_name(&mut w, &hlc, "avelino/outl", PageKind::Page).unwrap();
        let meta = page_meta(&w, id).unwrap();
        assert_eq!(meta.slug, "avelino-outl");
        assert_eq!(meta.title, "avelino/outl");
        // Calling again with the same human-typed name must return the
        // same node (idempotent on the slugified form, not the raw input).
        let second = open_or_create_by_name(&mut w, &hlc, "avelino/outl", PageKind::Page).unwrap();
        assert_eq!(id, second);
    }

    #[test]
    fn page_meta_prefers_title_property_over_node_text() {
        // Regression (issue #88): disk-sourced / ingested pages park the
        // human-readable name in the `title::` property
        // (`diff_to_ops_with_page_props` emits it as `Op::SetProp` key
        // "title") while the page root node's text stays empty. The
        // mobile / desktop autocomplete then rendered the slug because
        // `page_meta` read `block_text` only. `title::` must win.
        let (mut w, hlc) = ws();
        // Create with an empty node text, mimicking an ingested page.
        let id = open_or_create(&mut w, &hlc, "avelino-outl", "", PageKind::Page).unwrap();
        set_prop(
            &mut w,
            &hlc,
            id,
            "title",
            PropValue::Text("avelino/outl".to_string()),
        )
        .unwrap();
        let meta = page_meta(&w, id).unwrap();
        assert_eq!(meta.title, "avelino/outl");
        assert_eq!(meta.slug, "avelino-outl");
    }

    #[test]
    fn page_meta_falls_back_to_slug_when_no_title_or_text() {
        // No `title::` property and empty node text → slug is the last
        // resort so a title is never blank.
        let (mut w, hlc) = ws();
        let id = open_or_create(&mut w, &hlc, "orphan", "", PageKind::Page).unwrap();
        let meta = page_meta(&w, id).unwrap();
        assert_eq!(meta.title, "orphan");
    }

    #[test]
    fn open_or_create_is_idempotent() {
        let (mut w, hlc) = ws();
        let a = open_or_create(&mut w, &hlc, "foo", "Foo", PageKind::Page).unwrap();
        let b = open_or_create(&mut w, &hlc, "foo", "Foo Renamed", PageKind::Page).unwrap();
        assert_eq!(a, b);
        // Title is NOT updated by a second open_or_create; the caller
        // is expected to use edit_text if they want to rename.
        let pages = list_all(&w);
        assert_eq!(pages[0].title, "Foo");
    }

    #[test]
    fn journal_round_trip() {
        let (mut w, hlc) = ws();
        let date = NaiveDate::from_ymd_opt(2026, 5, 27).unwrap();
        let id = open_journal(&mut w, &hlc, date).unwrap();
        let meta = page_meta(&w, id).unwrap();
        assert_eq!(meta.slug, "2026-05-27");
        assert_eq!(meta.kind, PageKind::Journal);
        assert!(meta.title.contains("2026"));
    }

    #[test]
    fn migration_moves_legacy_blocks() {
        let (mut w, hlc) = ws();
        let a = append_block(&mut w, &hlc, None, Some("legacy 1")).unwrap();
        let b = append_block(&mut w, &hlc, None, Some("legacy 2")).unwrap();
        let moved = migrate_legacy_into_today(&mut w, &hlc).unwrap();
        assert_eq!(moved, 2);
        let today_id = open_today(&mut w, &hlc).unwrap();
        assert_eq!(w.tree().parent(a), Some(today_id));
        assert_eq!(w.tree().parent(b), Some(today_id));
    }

    // Person-typed page tests (`type:: person`, `@` mention resolution,
    // edge cases) live in `crate::person::tests`. Keep this module
    // focused on the page-model primitives (slug, kind, title, journal).
}

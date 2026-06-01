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

use chrono::{Duration, Local, NaiveDate};
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
    /// Human-readable title (the page node's text).
    pub title: String,
    /// `page` or `journal`.
    pub kind: PageKind,
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
    let title = workspace.block_text(id).unwrap_or_else(|| slug.clone());
    Some(PageMeta {
        id: id.to_string(),
        slug,
        title,
        kind,
    })
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

/// Open the page for `slug`, creating it if it doesn't exist yet.
///
/// The page's [`NodeId`] is derived deterministically from the slug
/// (see [`page_id_from_slug`]), so a peer that locally creates the
/// same slug ends up writing to the same node. iCloud sync then
/// merges the two creators' edits into one page instead of leaving
/// two competing copies.
pub fn open_or_create(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    slug: &str,
    title: &str,
    kind: PageKind,
) -> Result<NodeId, ActionError> {
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

/// Today's date in the user's local timezone.
pub fn today() -> NaiveDate {
    Local::now().date_naive()
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
}

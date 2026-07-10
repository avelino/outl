//! List templates defined in the workspace.
//!
//! A template is any page with a non-empty `template::` property.
//! The property value is the template's invocation name (what the
//! user types after `/template`).

use outl_core::id::NodeId;
use outl_core::property::PropValue;
use outl_core::workspace::Workspace;
use serde::Serialize;

use crate::page::{read_text_prop, SLUG_KEY};
use crate::template::{parse_param_list, PARAMS_KEY, TEMPLATE_KEY};
use crate::tree::children_of;

/// A template discovered in the workspace.
#[derive(Debug, Clone, Serialize)]
pub struct TemplateEntry {
    /// Invocation name (the value of `template::`).
    pub name: String,
    /// Page slug (conventionally `template-<name>`).
    pub slug: String,
    /// Stringified page [`NodeId`].
    pub page_id: String,
    /// Declared parameter names (from `params::`), empty when the
    /// template is structural-only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<String>,
    /// `true` when another page in the workspace shares this
    /// `template:: <name>`. Resolution picks the first in tree order,
    /// so a duplicate silently shadows the rest — surfacing the flag
    /// lets a client (doctor / picker) warn the user.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub duplicate: bool,
}

/// Find the page node whose `template::` property matches `name`.
///
/// Whitespace-only `template::` values are ignored, same as
/// [`list_templates`], so the two paths agree on what counts as a
/// template. When two pages share a name, the first in tree
/// (fractional-position) order wins — deterministic, and the same
/// order `list_templates` preserves among equal names. A duplicate name
/// silently shadows the rest, so this logs a `tracing::warn!` listing
/// the collision (and `list_templates` flags it on `TemplateEntry`) to
/// keep the shadowing visible to the user.
pub fn find_template_by_name(workspace: &Workspace, name: &str) -> Option<NodeId> {
    let matches: Vec<NodeId> = children_of(workspace, NodeId::root())
        .into_iter()
        .filter_map(
            |(id, _)| match workspace.tree().property(id, TEMPLATE_KEY)? {
                PropValue::Text(t) if !t.trim().is_empty() && t == name => Some(id),
                _ => None,
            },
        )
        .collect();

    if matches.len() > 1 {
        tracing::warn!(
            template = name,
            count = matches.len(),
            "multiple pages share `template:: {name}`; resolving to the first in tree order — \
             the rest are shadowed"
        );
    }

    matches.into_iter().next()
}

/// List every template in the workspace, sorted by name.
pub fn list_templates(workspace: &Workspace) -> Vec<TemplateEntry> {
    let mut entries: Vec<TemplateEntry> = children_of(workspace, NodeId::root())
        .into_iter()
        .filter_map(|(id, _)| {
            let name = match workspace.tree().property(id, TEMPLATE_KEY)? {
                PropValue::Text(t) if !t.trim().is_empty() => t.clone(),
                _ => return None,
            };
            let slug = read_text_prop(workspace, id, SLUG_KEY)?;
            let params = match workspace.tree().property(id, PARAMS_KEY) {
                Some(PropValue::Text(s)) => parse_param_list(s),
                _ => Vec::new(),
            };
            Some(TemplateEntry {
                name,
                slug,
                page_id: id.to_string(),
                params,
                duplicate: false,
            })
        })
        .collect();
    entries.sort_by(|a, b| a.name.cmp(&b.name));

    // Flag every entry whose name is shared by another page: resolution
    // picks the first in tree order, so the rest are silently shadowed.
    // Surfacing the flag lets a client warn the user (doctor / picker).
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for e in &entries {
        *counts.entry(e.name.as_str()).or_default() += 1;
    }
    let dupes: std::collections::HashSet<String> = counts
        .into_iter()
        .filter(|(_, n)| *n > 1)
        .map(|(name, _)| name.to_string())
        .collect();
    for e in &mut entries {
        e.duplicate = dupes.contains(&e.name);
    }

    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::page::set_property;
    use crate::page::{open_or_create as open_or_create_page, PageKind};
    use outl_core::hlc::HlcGenerator;
    use outl_core::id::ActorId;
    use outl_core::property::PropValue;

    fn ws() -> (Workspace, HlcGenerator) {
        let actor = ActorId::new();
        (
            Workspace::open_in_memory(actor).unwrap(),
            HlcGenerator::new(actor),
        )
    }

    #[test]
    fn empty_workspace_has_no_templates() {
        let (workspace, _hlc) = ws();
        assert!(list_templates(&workspace).is_empty());
    }

    #[test]
    fn finds_page_with_template_property() {
        let (mut workspace, hlc) = ws();
        let id = open_or_create_page(
            &mut workspace,
            &hlc,
            "template-interview",
            "Interview Template",
            PageKind::Page,
        )
        .unwrap();
        set_property(
            &mut workspace,
            &hlc,
            id,
            TEMPLATE_KEY,
            Some(PropValue::Text("interview".into())),
        )
        .unwrap();

        let templates = list_templates(&workspace);
        assert_eq!(templates.len(), 1);
        assert_eq!(templates[0].name, "interview");
        assert_eq!(templates[0].slug, "template-interview");
    }

    #[test]
    fn ignores_pages_without_template_property() {
        let (mut workspace, hlc) = ws();
        open_or_create_page(
            &mut workspace,
            &hlc,
            "random-page",
            "Random",
            PageKind::Page,
        )
        .unwrap();

        assert!(list_templates(&workspace).is_empty());
    }

    #[test]
    fn parses_params_property() {
        let (mut workspace, hlc) = ws();
        let id = open_or_create_page(
            &mut workspace,
            &hlc,
            "template-calc",
            "Calc",
            PageKind::Page,
        )
        .unwrap();
        set_property(
            &mut workspace,
            &hlc,
            id,
            TEMPLATE_KEY,
            Some(PropValue::Text("calc".into())),
        )
        .unwrap();
        set_property(
            &mut workspace,
            &hlc,
            id,
            PARAMS_KEY,
            Some(PropValue::Text("requested, offered".into())),
        )
        .unwrap();

        let templates = list_templates(&workspace);
        assert_eq!(templates[0].params, vec!["requested", "offered"]);
    }

    #[test]
    fn find_by_name_resolves_page_id() {
        let (mut workspace, hlc) = ws();
        let id = open_or_create_page(&mut workspace, &hlc, "template-1on1", "1:1", PageKind::Page)
            .unwrap();
        set_property(
            &mut workspace,
            &hlc,
            id,
            TEMPLATE_KEY,
            Some(PropValue::Text("1on1".into())),
        )
        .unwrap();

        let found = find_template_by_name(&workspace, "1on1");
        assert_eq!(found, Some(id));
    }

    #[test]
    fn duplicate_template_name_is_detectable() {
        let (mut workspace, hlc) = ws();
        for slug in ["template-dup-a", "template-dup-b"] {
            let id = open_or_create_page(&mut workspace, &hlc, slug, slug, PageKind::Page).unwrap();
            set_property(
                &mut workspace,
                &hlc,
                id,
                TEMPLATE_KEY,
                Some(PropValue::Text("dup".into())),
            )
            .unwrap();
        }

        // Both entries surface the collision so a client can warn.
        let templates = list_templates(&workspace);
        let dup: Vec<&TemplateEntry> = templates.iter().filter(|t| t.name == "dup").collect();
        assert_eq!(dup.len(), 2, "both duplicate pages are listed");
        assert!(
            dup.iter().all(|t| t.duplicate),
            "both entries flag the name collision"
        );

        // A unique name is never flagged.
        let unique_id = open_or_create_page(
            &mut workspace,
            &hlc,
            "template-solo",
            "solo",
            PageKind::Page,
        )
        .unwrap();
        set_property(
            &mut workspace,
            &hlc,
            unique_id,
            TEMPLATE_KEY,
            Some(PropValue::Text("solo".into())),
        )
        .unwrap();
        let templates = list_templates(&workspace);
        let solo = templates.iter().find(|t| t.name == "solo").unwrap();
        assert!(!solo.duplicate, "a unique name is not flagged");

        // Resolution still returns the first in tree order deterministically.
        assert!(find_template_by_name(&workspace, "dup").is_some());
    }
}

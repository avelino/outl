//! Callable template resolution.
//!
//! A callable template is a page with `template:: <name>` whose
//! outline contains a fenced code block. The ` ```call:<name> `
//! fence in a target block triggers resolution: find the template,
//! extract the first code block's language + source, and return it
//! for execution via `outl-exec`.

use outl_core::property::PropValue;
use outl_core::workspace::Workspace;
use serde::Serialize;

use crate::error::ActionError;
use crate::page::read_text_prop;
use crate::page::SLUG_KEY;
use crate::template::list::find_template_by_name;
use crate::template::{parse_param_list, PARAMS_KEY};
use crate::tree::walk_subtree;

/// Result of resolving a ` ```call:<name> ` block.
#[derive(Debug, Clone, Serialize)]
pub struct CallResolution {
    /// Template page slug.
    pub template_slug: String,
    /// Fence language detected in the template's code block
    /// (`"python"`, `"lisp"`, …).
    pub language: String,
    /// Raw source code extracted from the template's code block.
    pub source: String,
    /// Declared parameter names (from the template page's
    /// `params::` property).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<String>,
}

/// Resolve a callable template by name.
///
/// Finds the template page, walks its subtree looking for the first
/// fenced code block, and returns its language + source + declared
/// params. Returns [`ActionError::PageNotFound`] when the template
/// doesn't exist, or [`ActionError::Exec`] when the template page
/// has no code block.
pub fn resolve_call(
    workspace: &Workspace,
    template_name: &str,
) -> Result<CallResolution, ActionError> {
    let page_id = find_template_by_name(workspace, template_name)
        .ok_or_else(|| ActionError::PageNotFound(template_name.to_string()))?;

    let template_slug =
        read_text_prop(workspace, page_id, SLUG_KEY).unwrap_or_else(|| template_name.to_string());

    let params = match workspace.tree().property(page_id, PARAMS_KEY) {
        Some(PropValue::Text(s)) => parse_param_list(s),
        _ => Vec::new(),
    };

    // Walk the template page's subtree looking for the first code
    // block. Fence parsing is owned by `outl_exec::extract_fence`
    // (lowercased language, first info-string token) — reused here so
    // the template resolver and the runtime never drift on what counts
    // as a fence (see docs/contributing.md → Reuse-first).
    let mut found: Option<(String, String)> = None;
    walk_subtree(workspace, page_id, |id| {
        if found.is_some() {
            return false;
        }
        if let Some(text) = workspace.block_text(id) {
            if let Some(parts) = outl_exec::extract_fence(&text) {
                found = Some((parts.language, parts.body));
                return false;
            }
        }
        true
    });

    let (language, source) = found.ok_or_else(|| {
        ActionError::Exec(format!("template `{template_name}` has no code block"))
    })?;

    Ok(CallResolution {
        template_slug,
        language,
        source,
        params,
    })
}

/// The template name invoked by a ` ```call:<name> ` fence, or `None`
/// when `text` is not a call block.
///
/// This is the inverse of the `call:` language tag the exec path reads.
/// The backlinks panel uses it so a template page lists every block
/// that renders it, without the user hand-writing a `[[link]]`. Fence
/// parsing goes through the shared [`outl_exec::extract_fence`] (which
/// lowercases the info-string), mirroring how the runtime resolves the
/// call — so "what actually executes" and "what shows up in backlinks"
/// can't drift.
pub fn call_target_name(text: &str) -> Option<String> {
    let parts = outl_exec::extract_fence(text)?;
    let name = parts.language.strip_prefix("call:")?.trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// Prepend a `params` binding to a callable template's source so the
/// template body can read `params["key"]`.
///
/// Values are serialized with `serde_json`, so quotes, backslashes, and
/// newlines in a param value cannot break the generated program or
/// inject code — the call site is user (or shared-template) text, so
/// naive string interpolation is a real injection surface. The language
/// is canonicalized via [`outl_md::lang::canonical`], so aliases
/// (`py`, `python3`, `node`, `nodejs`, any casing) all get the params
/// prelude instead of silently running without it.
///
/// Languages without a known params convention get the source verbatim.
pub fn inject_call_params(language: &str, source: &str, params: &[(String, String)]) -> String {
    let obj: serde_json::Map<String, serde_json::Value> = params
        .iter()
        .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
        .collect();
    let json = serde_json::Value::Object(obj).to_string();

    match outl_md::lang::canonical(language).unwrap_or(language) {
        "js" => format!("var params = {json};\n{source}"),
        "python" => {
            // Embed the JSON text as a properly-escaped string literal
            // (double-quoted, JSON escapes are a subset of Python's) so
            // `json.loads(...)` receives it safely regardless of value
            // contents.
            let literal = serde_json::Value::String(json).to_string();
            format!("import json\nparams = json.loads({literal})\n{source}")
        }
        _ => source.to_string(),
    }
}

/// Parse the `key: value` body of a ` ```call:<name> ` block into
/// key-value pairs.
///
/// The body uses simple `key: value` lines (no nesting, no quoting).
/// This is intentionally minimal — complex param structures should
/// live in the template's code block, not in the call site.
pub fn parse_call_params(body: &str) -> Vec<(String, String)> {
    body.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let (key, value) = line.split_once(':')?;
            let key = key.trim().to_string();
            let value = value.trim().to_string();
            if key.is_empty() {
                None
            } else {
                Some((key, value))
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::append_block;
    use crate::page::{open_or_create as open_or_create_page, set_property, PageKind};
    use crate::TEMPLATE_KEY;
    use outl_core::hlc::HlcGenerator;
    use outl_core::id::ActorId;

    fn ws() -> (Workspace, HlcGenerator) {
        let actor = ActorId::new();
        (
            Workspace::open_in_memory(actor).unwrap(),
            HlcGenerator::new(actor),
        )
    }

    #[test]
    fn resolves_callable_template_with_code_block() {
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
        append_block(
            &mut workspace,
            &hlc,
            Some(id),
            Some("```python\nprint(1 + 1)\n```"),
        )
        .unwrap();

        let result = resolve_call(&workspace, "calc").unwrap();
        assert_eq!(result.language, "python");
        assert!(result.source.contains("print(1 + 1)"));
        assert_eq!(result.template_slug, "template-calc");
    }

    #[test]
    fn resolves_with_params() {
        let (mut workspace, hlc) = ws();
        let id = open_or_create_page(
            &mut workspace,
            &hlc,
            "template-salary",
            "Salary",
            PageKind::Page,
        )
        .unwrap();
        set_property(
            &mut workspace,
            &hlc,
            id,
            TEMPLATE_KEY,
            Some(PropValue::Text("salary".into())),
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
        append_block(
            &mut workspace,
            &hlc,
            Some(id),
            Some("```python\nresult = params['requested']\n```"),
        )
        .unwrap();

        let result = resolve_call(&workspace, "salary").unwrap();
        assert_eq!(result.params, vec!["requested", "offered"]);
    }

    #[test]
    fn fails_when_template_not_found() {
        let (workspace, _hlc) = ws();
        let result = resolve_call(&workspace, "nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn fails_when_no_code_block() {
        let (mut workspace, hlc) = ws();
        let id = open_or_create_page(
            &mut workspace,
            &hlc,
            "template-no-code",
            "No Code",
            PageKind::Page,
        )
        .unwrap();
        set_property(
            &mut workspace,
            &hlc,
            id,
            TEMPLATE_KEY,
            Some(PropValue::Text("no-code".into())),
        )
        .unwrap();
        append_block(&mut workspace, &hlc, Some(id), Some("just text")).unwrap();

        let result = resolve_call(&workspace, "no-code");
        assert!(result.is_err());
    }

    #[test]
    fn parses_call_params_body() {
        let body = "requested: 15000\noffered: 18000\n";
        let params = parse_call_params(body);
        assert_eq!(params.len(), 2);
        assert_eq!(params[0], ("requested".to_string(), "15000".to_string()));
        assert_eq!(params[1], ("offered".to_string(), "18000".to_string()));
    }

    #[test]
    fn parse_call_params_skips_comments() {
        let body = "# comment\nkey: value\n";
        let params = parse_call_params(body);
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn resolve_call_takes_first_code_fence() {
        let (mut workspace, hlc) = ws();
        let id = open_or_create_page(
            &mut workspace,
            &hlc,
            "template-multi",
            "Multi",
            PageKind::Page,
        )
        .unwrap();
        set_property(
            &mut workspace,
            &hlc,
            id,
            TEMPLATE_KEY,
            Some(PropValue::Text("multi".into())),
        )
        .unwrap();
        append_block(
            &mut workspace,
            &hlc,
            Some(id),
            Some("```python\nprint(1)\n```"),
        )
        .unwrap();
        append_block(
            &mut workspace,
            &hlc,
            Some(id),
            Some("```js\nconsole.log(2)\n```"),
        )
        .unwrap();

        let result = resolve_call(&workspace, "multi").unwrap();
        assert_eq!(result.language, "python", "first fence wins");
    }

    #[test]
    fn call_target_name_extracts_template() {
        assert_eq!(
            call_target_name("```call:calc-salary\npedido: 10\n```").as_deref(),
            Some("calc-salary")
        );
        // Not a call fence.
        assert_eq!(call_target_name("```python\nprint(1)\n```"), None);
        assert_eq!(call_target_name("plain text"), None);
        // Empty template name.
        assert_eq!(call_target_name("```call:\n```"), None);
    }

    #[test]
    fn inject_call_params_escapes_values() {
        // A value with a quote must not break the generated program.
        let params = vec![("note".to_string(), "he said \"hi\"".to_string())];
        let py = inject_call_params("python", "print(params['note'])", &params);
        assert!(py.starts_with("import json\nparams = json.loads("));
        // The dangerous quote is JSON-escaped, never bare in the source.
        assert!(!py.contains("he said \"hi\""));
        assert!(py.contains("he said"));

        let js = inject_call_params("javascript", "x", &params);
        assert!(js.starts_with("var params = {"));
    }

    #[test]
    fn inject_call_params_canonicalizes_aliases() {
        let params = vec![("x".to_string(), "1".to_string())];
        // `python3`/`node` are aliases — they must still get the prelude.
        assert!(inject_call_params("python3", "s", &params).contains("json.loads"));
        assert!(inject_call_params("node", "s", &params).starts_with("var params"));
        // Unknown language → source verbatim.
        assert_eq!(inject_call_params("ruby", "s", &params), "s");
    }
}

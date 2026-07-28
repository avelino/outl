//! Import report — the fidelity contract with the user.
//!
//! Every translation, fallback, and drop the pipeline performs is
//! counted here. The report doubles as the acceptance test for a
//! migration: run `--dry-run --json` against a real backup and the
//! numbers say exactly what would survive.

use serde::Serialize;
use std::collections::BTreeMap;

/// Summary of what an import produced (or would produce, on dry-run).
#[derive(Debug, Default, Serialize)]
pub struct ImportReport {
    /// Adapter id (`roam`, `logseq`, `obsidian`).
    pub source: String,
    /// Non-journal pages written.
    pub pages: usize,
    /// Journal pages written.
    pub journals: usize,
    /// Total outline blocks emitted.
    pub blocks: usize,
    /// `((uid))` refs resolved to a real `((blk-XXXXXX))` handle.
    pub refs_resolved: usize,
    /// Refs whose target page couldn't be handle-mapped — degraded to
    /// a `[[Page Title]]` link.
    pub refs_page_fallback: usize,
    /// Refs whose UID matches nothing in the source — left greppable
    /// as `((unresolved:uid))`.
    pub refs_unresolved: usize,
    /// `{{embed}}` of a block resolved to `!((blk-XXXXXX))`.
    pub embeds_resolved: usize,
    /// `{{embed}}` of a whole page — degraded to `[[Page]]` until
    /// page embeds land (issue #190).
    pub embeds_page_fallback: usize,
    /// `#[[Multi Word]]` tags rewritten to `[[Multi Word]]` page refs.
    pub tags_multiword_to_page_ref: usize,
    /// Source task-state counts (`TODO`, `DONE`, Logseq's `DOING`, …).
    pub tasks: BTreeMap<String, usize>,
    /// Components preserved verbatim, counted per kind
    /// (`query`, `table`, `kanban`, `latex`, …).
    pub components_verbatim: BTreeMap<String, usize>,
    /// Source queries translated to a ` ```query ` fence.
    pub queries_translated: usize,
    /// Org-style `DEADLINE:`/`SCHEDULED:` timestamps converted to
    /// `[[YYYY-MM-DD]]` date links (issue #63 model).
    pub org_dates_converted: usize,
    /// Page-level `key:: value` properties carried over.
    pub props_pages: usize,
    /// Block-level `key:: value` properties carried over.
    pub props_blocks: usize,
    /// Blocks whose collapsed/folded state landed as `Op::SetCollapsed`.
    pub collapsed_applied: usize,
    /// Blocks whose create/edit timestamps were dropped (default
    /// policy; re-run with `--preserve-timestamps` to keep them).
    pub timestamps_dropped: usize,
    /// Page-name collisions after slugify — disambiguated with a
    /// numeric suffix.
    pub slug_collisions: usize,
    /// Source-only metadata stripped on the way in (`id::` lines,
    /// `#+` directives, `:LOGBOOK:` drawers, dropped frontmatter
    /// keys, `logseq.*` properties).
    pub artifacts_stripped: usize,
    /// Source files skipped entirely, with the reason.
    pub skipped: Vec<SkippedFile>,
    /// Non-fatal fidelity warnings, with location.
    pub warnings: Vec<ImportWarning>,
}

/// A source file the import refused or had no mapping for.
#[derive(Debug, Serialize)]
pub struct SkippedFile {
    /// Source-relative path.
    pub path: String,
    /// Human-readable reason.
    pub reason: String,
}

/// One non-fatal fidelity warning.
#[derive(Debug, Serialize)]
pub struct ImportWarning {
    /// Page (title or date) the warning belongs to.
    pub page: String,
    /// What happened.
    pub detail: String,
}

impl ImportReport {
    /// Fresh report for the given adapter id.
    pub fn new(source: &str) -> Self {
        Self {
            source: source.to_string(),
            ..Self::default()
        }
    }

    /// Record a non-fatal fidelity warning.
    pub fn warn(&mut self, page: &str, detail: impl Into<String>) {
        self.warnings.push(ImportWarning {
            page: page.to_string(),
            detail: detail.into(),
        });
    }

    /// Bump a per-kind component counter.
    pub fn count_component(&mut self, kind: &str) {
        *self
            .components_verbatim
            .entry(kind.to_string())
            .or_insert(0) += 1;
    }

    /// Bump a per-state task counter.
    pub fn count_task(&mut self, state: &str) {
        *self.tasks.entry(state.to_string()).or_insert(0) += 1;
    }

    /// Pretty-print the report for humans.
    pub fn print_human(&self) {
        println!();
        println!("Import summary ({}):", self.source);
        println!("  pages:               {}", self.pages);
        println!("  journals:            {}", self.journals);
        println!("  blocks:              {}", self.blocks);
        println!(
            "  block refs:          {} resolved, {} page-fallback, {} unresolved",
            self.refs_resolved, self.refs_page_fallback, self.refs_unresolved
        );
        println!(
            "  embeds:              {} resolved, {} page-fallback",
            self.embeds_resolved, self.embeds_page_fallback
        );
        if !self.tasks.is_empty() {
            let states: Vec<String> = self
                .tasks
                .iter()
                .map(|(k, v)| format!("{k}: {v}"))
                .collect();
            println!("  tasks:               {}", states.join(", "));
        }
        println!(
            "  properties:          {} page-level, {} block-level",
            self.props_pages, self.props_blocks
        );
        if self.queries_translated > 0 {
            println!("  queries translated:  {}", self.queries_translated);
        }
        if !self.components_verbatim.is_empty() {
            let kinds: Vec<String> = self
                .components_verbatim
                .iter()
                .map(|(k, v)| format!("{k}: {v}"))
                .collect();
            println!("  components verbatim: {}", kinds.join(", "));
        }
        if self.tags_multiword_to_page_ref > 0 {
            println!(
                "  multi-word tags:     {} became [[page refs]]",
                self.tags_multiword_to_page_ref
            );
        }
        if self.org_dates_converted > 0 {
            println!(
                "  org dates:           {} became [[date]] links",
                self.org_dates_converted
            );
        }
        if self.collapsed_applied > 0 {
            println!("  collapsed blocks:    {}", self.collapsed_applied);
        }
        if self.timestamps_dropped > 0 {
            println!(
                "  timestamps dropped:  {} (re-run with --preserve-timestamps to keep)",
                self.timestamps_dropped
            );
        }
        if self.slug_collisions > 0 {
            println!("  slug collisions:     {}", self.slug_collisions);
        }
        if self.artifacts_stripped > 0 {
            println!(
                "  artifacts stripped:  {} (id:: lines, #+ directives, LOGBOOK drawers, dropped frontmatter keys)",
                self.artifacts_stripped
            );
        }
        for s in &self.skipped {
            println!("  skipped: {} ({})", s.path, s.reason);
        }
        if !self.warnings.is_empty() {
            println!("  warnings:            {}", self.warnings.len());
            for w in self.warnings.iter().take(20) {
                println!("    [{}] {}", w.page, w.detail);
            }
            if self.warnings.len() > 20 {
                println!(
                    "    … {} more (use --json for all)",
                    self.warnings.len() - 20
                );
            }
        }
    }
}

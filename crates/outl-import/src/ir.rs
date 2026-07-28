//! Typed intermediate representation every adapter targets.
//!
//! The IR is deliberately minimal: a construct only gets a variant
//! when the emitter must *transform or resolve* it. Anything that is
//! already valid outl markdown (`[[page]]`, `#tag`, `**bold**`,
//! plain links) travels inside [`Inline::Text`] verbatim — outl's own
//! parser tokenizes it after emission, so nothing is lost by keeping
//! it opaque here.

use chrono::{DateTime, NaiveDate, Utc};

/// A whole parsed source graph.
#[derive(Debug, Default)]
pub struct ImportGraph {
    /// Every page, journal or named.
    pub pages: Vec<ImportPage>,
}

/// Where a page lands in the workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageName {
    /// A daily note → `journals/YYYY-MM-DD.md`, no `title::`.
    Journal(NaiveDate),
    /// A named page → `pages/<slug>.md` with `title:: <name>`.
    Named(String),
}

/// One page of the source graph.
#[derive(Debug)]
pub struct ImportPage {
    /// Journal date or human title.
    pub name: PageName,
    /// Page-level `key:: value` properties (excluding `title::`,
    /// which the emitter owns).
    pub props: Vec<(String, String)>,
    /// Page body — outline tree or free markdown.
    pub body: PageBody,
    /// On-disk file stem override for named pages. Adapters with
    /// their own collision policy (Obsidian's path-derived suffixes)
    /// set it; `None` lets the emitter slugify the title.
    pub stem_override: Option<String>,
}

/// Page body shapes.
#[derive(Debug)]
pub enum PageBody {
    /// Structured outline (Roam, Logseq) — every block typed, refs
    /// resolvable, DFS-mapped to sidecar entries.
    Outline(Vec<ImportBlock>),
    /// Free markdown already valid in outl's dialect (Obsidian) —
    /// emitted verbatim after the property header. outl's permissive
    /// parser owns the block structure downstream; no UIDs, so the
    /// resolve pass never targets these pages.
    Raw(String),
}

/// One outline block.
#[derive(Debug, Default)]
pub struct ImportBlock {
    /// Source UID (Roam `uid`, Logseq `id::`). The key the resolve
    /// pass uses to mint `((blk-XXXXXX))` handles.
    pub uid: Option<String>,
    /// Block body.
    pub content: BlockContent,
    /// Child blocks.
    pub children: Vec<ImportBlock>,
    /// Task state, when the source marked one.
    pub task: Option<TaskState>,
    /// Heading level 1–3 (Roam's `heading` attribute) — emitted as a
    /// `#`/`##`/`###` markdown prefix.
    pub heading: Option<u8>,
    /// Folded in the source → lands as `Op::SetCollapsed`.
    pub collapsed: bool,
    /// Block-level `key:: value` properties — emitted as continuation
    /// property lines, not child blocks.
    pub props: Vec<(String, String)>,
    /// Source creation timestamp (kept only with
    /// `--preserve-timestamps`).
    pub created: Option<DateTime<Utc>>,
    /// Source last-edit timestamp (same policy).
    pub edited: Option<DateTime<Utc>>,
}

/// Block body shapes.
#[derive(Debug)]
pub enum BlockContent {
    /// Regular text, tokenized. `Text` runs may contain newlines —
    /// the emitter renders them as continuation lines.
    Inline(Vec<Inline>),
    /// A fenced code block (` ```lang … ``` `).
    Code {
        /// Fence info-string, if any.
        lang: Option<String>,
        /// Fence body, without the fence lines.
        body: String,
    },
    /// Content the adapter could not confidently tokenize — preserved
    /// byte-for-byte. Never drop user content.
    Verbatim(String),
}

impl Default for BlockContent {
    fn default() -> Self {
        Self::Inline(Vec::new())
    }
}

/// Inline tokens — only what the emitter must transform or resolve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inline {
    /// Verbatim run, already valid outl markdown.
    Text(String),
    /// `` `code` `` span — protected from dialect rewrites.
    CodeSpan(String),
    /// `((uid))` block reference, optionally aliased
    /// (`[alias](((uid)))`). Resolved to `((blk-XXXXXX))`.
    BlockRef {
        /// Source UID.
        uid: String,
        /// Display alias, when the source had one.
        alias: Option<String>,
    },
    /// `{{embed}}` of a block or page.
    Embed(EmbedTarget),
    /// A `{{…}}` component with no outl equivalent — preserved
    /// verbatim and counted in the report.
    Component {
        /// Classified kind (for the report).
        kind: ComponentKind,
        /// The raw source text, braces included.
        raw: String,
    },
}

/// What an embed points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmbedTarget {
    /// Block embed → `!((blk-XXXXXX))`.
    Block(String),
    /// Page embed → `[[Page]]` fallback until issue #190 lands.
    Page(String),
}

/// Component classification. `Query` is the only kind the pipeline
/// treats structurally (whole-block queries may translate to a
/// ` ```query ` fence); everything else is verbatim passthrough and
/// the *report key* carries the taxonomy (`table`, `latex`, …).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComponentKind {
    /// `{{query}}` — may translate; counted where the decision lands.
    Query,
    /// Any other component — preserved verbatim, counted by name.
    Other,
}

/// Task states across supported sources. Roam only emits
/// `Todo`/`Done`; the rest are Logseq's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    /// Open task.
    Todo,
    /// In progress (Logseq `DOING`).
    Doing,
    /// Scheduled for today (Logseq `NOW`).
    Now,
    /// Deferred (Logseq `LATER`).
    Later,
    /// Blocked (Logseq `WAITING`).
    Waiting,
    /// Completed.
    Done,
    /// Abandoned (Logseq `CANCELED`).
    Canceled,
}

impl TaskState {
    /// The outl text prefix this state maps to.
    pub fn outl_prefix(self) -> &'static str {
        match self {
            Self::Todo | Self::Doing | Self::Now | Self::Later | Self::Waiting => "TODO",
            Self::Done | Self::Canceled => "DONE",
        }
    }

    /// Extra `state::` property preserving the source nuance, when the
    /// outl prefix alone is lossy.
    pub fn state_prop(self) -> Option<&'static str> {
        match self {
            Self::Todo | Self::Done => None,
            Self::Doing => Some("doing"),
            Self::Now => Some("now"),
            Self::Later => Some("later"),
            Self::Waiting => Some("waiting"),
            Self::Canceled => Some("canceled"),
        }
    }

    /// Report key (source-side state name).
    pub fn report_key(self) -> &'static str {
        match self {
            Self::Todo => "TODO",
            Self::Doing => "DOING",
            Self::Now => "NOW",
            Self::Later => "LATER",
            Self::Waiting => "WAITING",
            Self::Done => "DONE",
            Self::Canceled => "CANCELED",
        }
    }
}

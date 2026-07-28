//! Source adapters. One module per dialect.

pub mod logseq;
pub mod obsidian;
pub mod roam;
pub(crate) mod scan;

pub use logseq::LogseqAdapter;
pub use obsidian::ObsidianAdapter;
pub use roam::RoamAdapter;

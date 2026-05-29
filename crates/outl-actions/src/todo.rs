//! TODO/DONE state, encoded as a prefix on a block's text.
//!
//! The TUI established this convention and we keep it for wire-format
//! compatibility: `"TODO foo"` and `"DONE foo"` are the only valid
//! marker shapes, separated from the body by a single space.

/// Wire prefix for an open task. Six characters including the trailing
/// space, so consumers can rely on `chars().count() == 5` for cursor
/// math without re-deriving it.
pub const TODO_PREFIX: &str = "TODO ";
/// Wire prefix for a completed task. Same length as [`TODO_PREFIX`].
pub const DONE_PREFIX: &str = "DONE ";

/// Recognised TODO states. The order also defines the cycle order in
/// [`cycle_todo`]: `None → TODO → DONE → None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TodoState {
    /// The block is an open task.
    Todo,
    /// The block is a completed task.
    Done,
}

impl TodoState {
    /// Stringified wire form used in block text and rendered markdown.
    pub fn as_str(self) -> &'static str {
        match self {
            TodoState::Todo => "TODO",
            TodoState::Done => "DONE",
        }
    }

    /// Prefix used when this state lives inline in a block's text,
    /// e.g. `"TODO "` or `"DONE "`.
    pub fn prefix(self) -> &'static str {
        match self {
            TodoState::Todo => TODO_PREFIX,
            TodoState::Done => DONE_PREFIX,
        }
    }
}

/// Split a block's raw text into `(state, body)`. The body never
/// includes the prefix or its trailing space.
pub fn split_todo(raw: &str) -> (Option<TodoState>, &str) {
    if let Some(rest) = raw.strip_prefix("TODO ") {
        (Some(TodoState::Todo), rest)
    } else if let Some(rest) = raw.strip_prefix("DONE ") {
        (Some(TodoState::Done), rest)
    } else {
        (None, raw)
    }
}

/// Cycle the TODO state of `raw` to the next stop. Returns the new
/// text, ready to be stored as the block's content.
pub fn cycle_todo(raw: &str) -> String {
    let (state, body) = split_todo(raw);
    match state {
        None => format!("TODO {body}"),
        Some(TodoState::Todo) => format!("DONE {body}"),
        Some(TodoState::Done) => body.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_recognises_both_markers() {
        assert_eq!(
            split_todo("TODO write report"),
            (Some(TodoState::Todo), "write report")
        );
        assert_eq!(
            split_todo("DONE shipped it"),
            (Some(TodoState::Done), "shipped it")
        );
        assert_eq!(split_todo("plain block"), (None, "plain block"));
    }

    #[test]
    fn cycle_walks_through_three_states() {
        let s0 = "deploy frontend";
        let s1 = cycle_todo(s0);
        let s2 = cycle_todo(&s1);
        let s3 = cycle_todo(&s2);
        assert_eq!(s1, "TODO deploy frontend");
        assert_eq!(s2, "DONE deploy frontend");
        assert_eq!(s3, "deploy frontend");
    }
}

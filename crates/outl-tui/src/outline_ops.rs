//! Pure AST manipulation helpers used by the TUI's `App`.
//!
//! These operate on the in-memory `Vec<OutlineNode>` (the user's
//! page in flight). They have no I/O, no workspace state, and no
//! ratatui dependency — they're the kind of thing a future Tauri or
//! mobile client would call too, the same way they call
//! `outl_md::tokenize`.
//!
//! All paths are vectors of child indices, DFS-preorder. `path[0]` is
//! the index of the top-level block, `path[1]` the index inside its
//! children, and so on. `flat_index` is a single counter that walks
//! the same DFS preorder, useful for "the user's selection cursor".
//!
//! Every public function in this module is tested below.

use outl_md::parse::OutlineNode;

/// Count of all nodes in the (possibly nested) outline.
pub fn flat_count(blocks: &[OutlineNode]) -> usize {
    blocks.iter().map(|b| 1 + flat_count(&b.children)).sum()
}

/// Return the path of indices to reach the block at `target_index`
/// in DFS preorder. `None` if the index is out of range.
pub fn path_for_index(blocks: &[OutlineNode], target: usize) -> Option<Vec<usize>> {
    let mut cursor = 0;
    walk_path(blocks, target, &mut cursor, &mut Vec::new())
}

fn walk_path(
    blocks: &[OutlineNode],
    target: usize,
    cursor: &mut usize,
    stack: &mut Vec<usize>,
) -> Option<Vec<usize>> {
    for (i, b) in blocks.iter().enumerate() {
        stack.push(i);
        if *cursor == target {
            return Some(stack.clone());
        }
        *cursor += 1;
        if let Some(path) = walk_path(&b.children, target, cursor, stack) {
            return Some(path);
        }
        stack.pop();
    }
    None
}

/// Reverse of [`path_for_index`]: given a path, return the flat index.
pub fn index_for_path(blocks: &[OutlineNode], path: &[usize]) -> Option<usize> {
    let mut cursor = 0;
    walk_index_for_path(blocks, path, 0, &mut cursor)
}

fn walk_index_for_path(
    blocks: &[OutlineNode],
    path: &[usize],
    depth: usize,
    cursor: &mut usize,
) -> Option<usize> {
    if depth >= path.len() {
        return None;
    }
    let target = path[depth];
    for (i, b) in blocks.iter().enumerate() {
        if i == target {
            if depth + 1 == path.len() {
                return Some(*cursor);
            }
            *cursor += 1;
            return walk_index_for_path(&b.children, path, depth + 1, cursor);
        } else {
            *cursor += 1 + flat_count(&b.children);
        }
    }
    None
}

/// Borrow the node at a path. `None` if any segment is out of range.
pub fn node_at_path<'a>(blocks: &'a [OutlineNode], path: &[usize]) -> Option<&'a OutlineNode> {
    let mut current = blocks;
    let mut node: Option<&OutlineNode> = None;
    for &idx in path {
        let n = current.get(idx)?;
        node = Some(n);
        current = &n.children;
    }
    node
}

/// Mutable variant of [`node_at_path`].
pub fn node_at_path_mut<'a>(
    blocks: &'a mut [OutlineNode],
    path: &[usize],
) -> Option<&'a mut OutlineNode> {
    let mut current = blocks;
    for (depth, &idx) in path.iter().enumerate() {
        if depth + 1 == path.len() {
            return current.get_mut(idx);
        }
        current = &mut current.get_mut(idx)?.children;
    }
    None
}

/// Number of descendants directly nested under the node at `path`.
pub fn descendants_count_at_path(blocks: &[OutlineNode], path: &[usize]) -> usize {
    node_at_path(blocks, path)
        .map(|n| flat_count(&n.children))
        .unwrap_or(0)
}

/// Insert a fresh empty block as a sibling immediately *after* `path`.
pub fn insert_sibling_after(blocks: &mut Vec<OutlineNode>, path: &[usize]) {
    if path.is_empty() {
        blocks.push(OutlineNode::default());
        return;
    }
    let (last, parent_path) = path.split_last().unwrap();
    let siblings = siblings_mut(blocks, parent_path);
    let pos = last + 1;
    siblings.insert(pos, OutlineNode::default());
}

/// Insert a fresh empty block as a sibling immediately *before* `path`.
pub fn insert_sibling_before(blocks: &mut Vec<OutlineNode>, path: &[usize]) {
    if path.is_empty() {
        blocks.insert(0, OutlineNode::default());
        return;
    }
    let (last, parent_path) = path.split_last().unwrap();
    let siblings = siblings_mut(blocks, parent_path);
    siblings.insert(*last, OutlineNode::default());
}

/// Borrow the sibling list of a path (i.e. the parent's children).
pub fn siblings_mut<'a>(
    blocks: &'a mut Vec<OutlineNode>,
    parent_path: &[usize],
) -> &'a mut Vec<OutlineNode> {
    let mut current = blocks;
    for &idx in parent_path {
        current = &mut current[idx].children;
    }
    current
}

/// Indent: become the last child of the previous sibling. Returns the
/// new path of the moved block, or `None` if there is no previous
/// sibling (already at the top of its parent).
pub fn indent_at_path(blocks: &mut Vec<OutlineNode>, path: &[usize]) -> Option<Vec<usize>> {
    let (last_idx, parent_path) = path.split_last()?;
    if *last_idx == 0 {
        return None;
    }
    let siblings = siblings_mut(blocks, parent_path);
    let node = siblings.remove(*last_idx);
    let prev = &mut siblings[*last_idx - 1];
    let new_idx = prev.children.len();
    prev.children.push(node);
    let mut new_path = parent_path.to_vec();
    new_path.push(*last_idx - 1);
    new_path.push(new_idx);
    Some(new_path)
}

/// Outdent: become the next sibling of the parent. Returns the new
/// path, or `None` if already at the top level.
pub fn outdent_at_path(blocks: &mut Vec<OutlineNode>, path: &[usize]) -> Option<Vec<usize>> {
    if path.len() < 2 {
        return None;
    }
    let (last_idx, parent_path) = path.split_last()?;
    let (parent_idx, grandparent_path) = parent_path.split_last()?;
    let parent_idx = *parent_idx;
    let last_idx = *last_idx;
    let node = {
        let siblings = siblings_mut(blocks, parent_path);
        siblings.remove(last_idx)
    };
    let grandparent_siblings = siblings_mut(blocks, grandparent_path);
    grandparent_siblings.insert(parent_idx + 1, node);
    let mut new_path = grandparent_path.to_vec();
    new_path.push(parent_idx + 1);
    Some(new_path)
}

/// Flatten the subtree rooted at `root` into a DFS-ordered sequence of
/// paths relative to that root. The first entry is always the empty
/// path `[]` (the root itself); subsequent entries descend into
/// children in order.
///
/// Used by the inline backlinks section so `j`/`k` can step through a
/// referencing block and its children the same way the main outline
/// stepping works on `app.page`.
pub fn flatten_backlink_subtree(root: &OutlineNode) -> Vec<Vec<usize>> {
    let mut out = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    out.push(stack.clone()); // the root, path = []
    walk_subtree(root, &mut stack, &mut out);
    out
}

fn walk_subtree(node: &OutlineNode, stack: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
    for (i, child) in node.children.iter().enumerate() {
        stack.push(i);
        out.push(stack.clone());
        walk_subtree(child, stack, out);
        stack.pop();
    }
}

/// Delete the node at `path`. Silently no-ops on out-of-range or root.
pub fn delete_at_path(blocks: &mut Vec<OutlineNode>, path: &[usize]) {
    if path.is_empty() {
        return;
    }
    let (last_idx, parent_path) = path.split_last().unwrap();
    let siblings = siblings_mut(blocks, parent_path);
    if *last_idx < siblings.len() {
        siblings.remove(*last_idx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(text: &str) -> OutlineNode {
        OutlineNode {
            text: text.into(),
            ..Default::default()
        }
    }

    #[test]
    fn flat_count_counts_nested_blocks() {
        let blocks = vec![
            OutlineNode {
                text: "a".into(),
                children: vec![block("a1"), block("a2")],
                ..Default::default()
            },
            block("b"),
        ];
        assert_eq!(flat_count(&blocks), 4);
    }

    #[test]
    fn path_for_index_round_trips() {
        let blocks = vec![
            OutlineNode {
                text: "a".into(),
                children: vec![block("a1"), block("a2")],
                ..Default::default()
            },
            block("b"),
        ];
        for i in 0..flat_count(&blocks) {
            let path = path_for_index(&blocks, i).unwrap();
            let back = index_for_path(&blocks, &path).unwrap();
            assert_eq!(back, i, "round-trip failed at index {i}");
        }
    }

    #[test]
    fn indent_makes_block_child_of_previous_sibling() {
        let mut blocks = vec![block("a"), block("b")];
        let new_path = indent_at_path(&mut blocks, &[1]).unwrap();
        assert_eq!(new_path, vec![0, 0]);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].children.len(), 1);
        assert_eq!(blocks[0].children[0].text, "b");
    }

    #[test]
    fn indent_first_block_is_noop() {
        let mut blocks = vec![block("a")];
        assert!(indent_at_path(&mut blocks, &[0]).is_none());
    }

    #[test]
    fn outdent_promotes_child_to_grandparent_level() {
        let mut blocks = vec![OutlineNode {
            text: "a".into(),
            children: vec![block("a1")],
            ..Default::default()
        }];
        let new_path = outdent_at_path(&mut blocks, &[0, 0]).unwrap();
        assert_eq!(new_path, vec![1]);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[1].text, "a1");
    }

    #[test]
    fn outdent_top_level_is_noop() {
        let mut blocks = vec![block("a")];
        assert!(outdent_at_path(&mut blocks, &[0]).is_none());
    }

    #[test]
    fn insert_sibling_after_inserts_at_correct_position() {
        let mut blocks = vec![block("a"), block("b")];
        insert_sibling_after(&mut blocks, &[0]);
        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0].text, "a");
        assert_eq!(blocks[1].text, "");
        assert_eq!(blocks[2].text, "b");
    }

    #[test]
    fn delete_removes_block_and_descendants() {
        let mut blocks = vec![
            OutlineNode {
                text: "a".into(),
                children: vec![block("a1")],
                ..Default::default()
            },
            block("b"),
        ];
        delete_at_path(&mut blocks, &[0]);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].text, "b");
    }

    #[test]
    fn flatten_backlink_subtree_returns_dfs_paths() {
        let root = OutlineNode {
            text: "root".into(),
            children: vec![
                OutlineNode {
                    text: "a".into(),
                    children: vec![block("a1"), block("a2")],
                    ..Default::default()
                },
                block("b"),
            ],
            ..Default::default()
        };
        // DFS preorder: root, a, a1, a2, b
        assert_eq!(
            flatten_backlink_subtree(&root),
            vec![
                vec![],     // root
                vec![0],    // a
                vec![0, 0], // a1
                vec![0, 1], // a2
                vec![1],    // b
            ]
        );
    }

    #[test]
    fn flatten_backlink_subtree_leaf_returns_just_root() {
        let leaf = block("only-me");
        assert_eq!(flatten_backlink_subtree(&leaf), vec![Vec::<usize>::new()]);
    }

    #[test]
    fn descendants_count_handles_nested() {
        let blocks = vec![OutlineNode {
            text: "a".into(),
            children: vec![block("a1"), block("a2")],
            ..Default::default()
        }];
        assert_eq!(descendants_count_at_path(&blocks, &[0]), 2);
        assert_eq!(descendants_count_at_path(&blocks, &[0, 0]), 0);
    }
}

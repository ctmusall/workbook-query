//! Addressing nodes of a condition tree by path (indexes from the root group down).

use wbquery_core::conditions::{Group, Node};

/// The node at `path`: None for the root (an empty path) or a path that leads nowhere.
pub fn node<'a>(root: &'a Group, path: &[usize]) -> Option<&'a Node> {
    let (last, parents) = path.split_last()?;
    group(root, parents)?.items.get(*last)
}

/// The group at `path` (the root for an empty path); None when the path doesn't lead to a group.
pub fn group<'a>(root: &'a Group, path: &[usize]) -> Option<&'a Group> {
    let mut g = root;
    for &i in path {
        match g.items.get(i)? {
            Node::Group(sub) => g = sub,
            Node::Cond(_) => return None,
        }
    }
    Some(g)
}

pub fn group_mut<'a>(root: &'a mut Group, path: &[usize]) -> Option<&'a mut Group> {
    let mut g = root;
    for &i in path {
        match g.items.get_mut(i)? {
            Node::Group(sub) => g = sub,
            Node::Cond(_) => return None,
        }
    }
    Some(g)
}

/// Whether `path` names a group (the root counts).
pub fn is_group(root: &Group, path: &[usize]) -> bool {
    path.is_empty() || matches!(node(root, path), Some(Node::Group(_)))
}

/// Rename a source in every condition that names it.
pub fn rename_source(g: &mut Group, old: &str, new: &str) {
    for c in g.conditions_mut() {
        if c.source == old {
            c.source = new.to_string();
        }
    }
}

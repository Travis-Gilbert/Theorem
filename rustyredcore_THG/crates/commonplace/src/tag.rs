//! Tags: lightweight labels on items (plan unit F1).
//!
//! A [`Tag`] is a first-class graph node keyed by a stable slug derived from its
//! name, so the same label always resolves to one node (exact-name dedup). F2's
//! embedding-based near-duplicate resolution layers on top of this.

use serde::{Deserialize, Serialize};

/// A label that can be attached to many items.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tag {
    pub id: String,
    pub name: String,
}

/// The stable node id for a tag name. Lowercases, trims, and collapses runs of
/// non-alphanumeric characters to a single `-`, so "Machine Learning" and
/// "machine-learning" map to the same `tag:machine-learning` node.
pub fn tag_id(name: &str) -> String {
    let mut slug = String::new();
    let mut last_dash = false;
    for ch in name.trim().to_lowercase().chars() {
        if ch.is_alphanumeric() {
            slug.push(ch);
            last_dash = false;
        } else if !last_dash {
            slug.push('-');
            last_dash = true;
        }
    }
    let slug = slug.trim_matches('-');
    format!("tag:{}", if slug.is_empty() { "untitled" } else { slug })
}

#[cfg(test)]
mod tests {
    use super::tag_id;

    #[test]
    fn tag_id_is_stable_across_casing_and_separators() {
        assert_eq!(tag_id("Machine Learning"), "tag:machine-learning");
        assert_eq!(tag_id("machine-learning"), "tag:machine-learning");
        assert_eq!(tag_id("  ML  "), "tag:ml");
        assert_eq!(tag_id("a / b"), "tag:a-b");
        assert_eq!(tag_id("!!!"), "tag:untitled");
    }
}

//! The message tree of a session and the path the screen shows through it.

use std::collections::HashMap;

use workcraft_core::Message;

/// One step of the visible path, with where it sits among its siblings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub id: String,
    /// Parent id; `None` for a root.
    pub parent: Option<String>,
    /// Zero-based position among siblings, ordered by time.
    pub index: usize,
    pub count: usize,
    /// Sibling ids in order, so the UI can switch to one.
    pub siblings: Vec<String>,
}

/// Walk from the root down. At each node follow `choice[node]` if it names a child, else the newest child.
/// `choice` is keyed by parent id, with `""` for the root level.
#[must_use]
pub fn visible_path(messages: &[Message], choice: &HashMap<String, String>) -> Vec<Step> {
    let ids: std::collections::HashSet<&str> = messages.iter().map(|m| m.id.as_str()).collect();
    let mut children: HashMap<&str, Vec<&Message>> = HashMap::new();
    for m in messages {
        let key = match m.parent.as_deref() {
            Some(p) if ids.contains(p) => p,
            _ => "",
        };
        children.entry(key).or_default().push(m);
    }
    for v in children.values_mut() {
        v.sort_by_key(|m| m.ts);
    }

    let mut path = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut key = "";
    while let Some(sibs) = children.get(key) {
        let pick = choice
            .get(key)
            .and_then(|c| sibs.iter().position(|m| &m.id == c))
            .unwrap_or(sibs.len() - 1);
        let m = sibs[pick];
        if !seen.insert(m.id.as_str()) {
            break;
        }
        path.push(Step {
            id: m.id.clone(),
            parent: (!key.is_empty()).then(|| key.to_string()),
            index: pick,
            count: sibs.len(),
            siblings: sibs.iter().map(|s| s.id.clone()).collect(),
        });
        key = m.id.as_str();
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(id: &str, parent: Option<&str>, ts: i64) -> Message {
        let mut x = Message::user(id).with_parent(parent);
        x.id = id.to_string();
        x.ts = ts;
        x
    }

    fn ids(p: &[Step]) -> Vec<&str> {
        p.iter().map(|s| s.id.as_str()).collect()
    }

    #[test]
    fn follows_the_newest_branch_by_default() {
        let msgs = vec![
            m("u1", None, 1),
            m("a1", Some("u1"), 2),
            m("u2", Some("a1"), 3),
            m("u2b", Some("a1"), 4),
            m("a2b", Some("u2b"), 5),
        ];
        let p = visible_path(&msgs, &HashMap::new());
        assert_eq!(ids(&p), vec!["u1", "a1", "u2b", "a2b"]);
        assert_eq!((p[2].index, p[2].count), (1, 2));
        assert_eq!(p[2].siblings, vec!["u2", "u2b"]);
    }

    #[test]
    fn a_choice_switches_the_branch() {
        let msgs = vec![
            m("u1", None, 1),
            m("a1", Some("u1"), 2),
            m("u2", Some("a1"), 3),
            m("u2b", Some("a1"), 4),
        ];
        let choice = HashMap::from([("a1".to_string(), "u2".to_string())]);
        assert_eq!(ids(&visible_path(&msgs, &choice)), vec!["u1", "a1", "u2"]);
        let bad = HashMap::from([("a1".to_string(), "ghost".to_string())]);
        assert_eq!(
            ids(&visible_path(&msgs, &bad)),
            vec!["u1", "a1", "u2b"],
            "an unknown choice falls back to the newest"
        );
    }

    #[test]
    fn empty_orphans_and_cycles_are_safe() {
        assert!(visible_path(&[], &HashMap::new()).is_empty());
        // An orphan (parent missing) counts as a root.
        let p = visible_path(&[m("x", Some("gone"), 1)], &HashMap::new());
        assert_eq!(ids(&p), vec!["x"]);
        // A cycle has no root, so nothing is shown and nothing hangs.
        let cyc = vec![m("a", Some("b"), 1), m("b", Some("a"), 2)];
        assert!(visible_path(&cyc, &HashMap::new()).is_empty());
    }
}

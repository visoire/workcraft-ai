//! Sessions as JSONL files.
//!
//! `<dir>/<id>.jsonl`: the first line is the header, every other line is one message. Messages are only
//! ever appended. A message carries its `parent`, so a branch is a new child of an older message and the
//! active path is found by walking parents from a leaf. A damaged line is skipped and counted, never fatal.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use workcraft_core::{Error, Message, ModelRef, Result, new_id, now_ms};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Header {
    pub id: String,
    pub title: String,
    pub created: i64,
    #[serde(default)]
    pub model: Option<ModelRef>,
    /// `single` or `team`.
    #[serde(default = "single")]
    pub mode: String,
}

fn single() -> String {
    "single".to_string()
}

/// One line of a session file.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Line {
    Header(Header),
    Message(Message),
    /// A later header line replaces the title or model.
    Meta {
        title: Option<String>,
        model: Option<ModelRef>,
        mode: Option<String>,
    },
}

#[derive(Debug, Clone)]
pub struct Session {
    pub header: Header,
    pub messages: Vec<Message>,
    /// Lines that could not be read.
    pub skipped: usize,
}

impl Session {
    /// The messages from the root to `leaf`, oldest first.
    #[must_use]
    pub fn path_to(&self, leaf: &str) -> Vec<&Message> {
        let mut out: Vec<&Message> = Vec::new();
        let mut cur = Some(leaf.to_string());
        while let Some(id) = cur {
            let Some(m) = self.messages.iter().find(|m| m.id == id) else {
                break;
            };
            // A cycle would be a corrupt file; stop instead of looping.
            if out.iter().any(|x| x.id == m.id) {
                break;
            }
            out.push(m);
            cur = m.parent.clone();
        }
        out.reverse();
        out
    }

    /// The newest message, the default tip of the conversation.
    #[must_use]
    pub fn latest(&self) -> Option<&Message> {
        self.messages.iter().max_by_key(|m| m.ts)
    }

    /// The newest message at the end of the main line: the last message that has no children.
    #[must_use]
    pub fn tip(&self) -> Option<&Message> {
        self.messages
            .iter()
            .filter(|m| {
                !self
                    .messages
                    .iter()
                    .any(|c| c.parent.as_deref() == Some(m.id.as_str()))
            })
            .max_by_key(|m| m.ts)
    }

    #[must_use]
    pub fn children_of(&self, id: &str) -> Vec<&Message> {
        self.messages
            .iter()
            .filter(|m| m.parent.as_deref() == Some(id))
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    pub id: String,
    pub title: String,
    pub created: i64,
    pub updated: i64,
    pub messages: usize,
}

pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self> {
        let dir = dir.into();
        fs::create_dir_all(&dir)?;
        Ok(Self { dir })
    }

    fn file(&self, id: &str) -> Result<PathBuf> {
        if id.is_empty()
            || !id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(Error::Invalid(format!("bad session id '{id}'")));
        }
        Ok(self.dir.join(format!("{id}.jsonl")))
    }

    pub fn create(&self, title: &str, model: Option<ModelRef>, mode: &str) -> Result<Header> {
        let header = Header {
            id: new_id(),
            title: if title.is_empty() {
                "New chat".to_string()
            } else {
                title.to_string()
            },
            created: now_ms(),
            model,
            mode: mode.to_string(),
        };
        append_line(&self.file(&header.id)?, &Line::Header(header.clone()))?;
        Ok(header)
    }

    pub fn append(&self, session: &str, message: &Message) -> Result<()> {
        let path = self.file(session)?;
        if !path.exists() {
            return Err(Error::NotFound(format!("session {session}")));
        }
        append_line(&path, &Line::Message(message.clone()))
    }

    pub fn set_meta(
        &self,
        session: &str,
        title: Option<String>,
        model: Option<ModelRef>,
        mode: Option<String>,
    ) -> Result<()> {
        let path = self.file(session)?;
        if !path.exists() {
            return Err(Error::NotFound(format!("session {session}")));
        }
        append_line(&path, &Line::Meta { title, model, mode })
    }

    pub fn load(&self, session: &str) -> Result<Session> {
        let path = self.file(session)?;
        if !path.exists() {
            return Err(Error::NotFound(format!("session {session}")));
        }
        load_file(&path)
    }

    pub fn delete(&self, session: &str) -> Result<()> {
        let path = self.file(session)?;
        if path.exists() {
            fs::remove_file(path)?;
        }
        Ok(())
    }

    /// Newest first. Unreadable files are left out.
    pub fn list(&self) -> Result<Vec<Summary>> {
        let mut out = Vec::new();
        for entry in fs::read_dir(&self.dir)? {
            let Ok(entry) = entry else { continue };
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            if let Ok(s) = load_file(&path) {
                out.push(Summary {
                    id: s.header.id.clone(),
                    title: s.header.title.clone(),
                    created: s.header.created,
                    updated: s
                        .messages
                        .iter()
                        .map(|m| m.ts)
                        .max()
                        .unwrap_or(s.header.created),
                    messages: s.messages.len(),
                });
            }
        }
        out.sort_by_key(|s| std::cmp::Reverse(s.updated));
        Ok(out)
    }
}

fn append_line(path: &Path, line: &Line) -> Result<()> {
    let mut text = serde_json::to_string(line)?;
    text.push('\n');
    let mut f = OpenOptions::new().create(true).append(true).open(path)?;
    f.write_all(text.as_bytes())?;
    Ok(())
}

fn load_file(path: &Path) -> Result<Session> {
    let raw = fs::read(path)?;
    let text = String::from_utf8_lossy(&raw);
    let mut header: Option<Header> = None;
    let mut messages = Vec::new();
    let mut skipped = 0;
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        match serde_json::from_str::<Line>(line) {
            Ok(Line::Header(h)) => header = Some(h),
            Ok(Line::Message(m)) => messages.push(m),
            Ok(Line::Meta { title, model, mode }) => {
                if let Some(h) = header.as_mut() {
                    if let Some(t) = title {
                        h.title = t;
                    }
                    if model.is_some() {
                        h.model = model;
                    }
                    if let Some(m) = mode {
                        h.mode = m;
                    }
                }
            }
            Err(_) => skipped += 1,
        }
    }
    let header = header.ok_or_else(|| Error::Store(format!("{} has no header", path.display())))?;
    Ok(Session {
        header,
        messages,
        skipped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("wc-store-{}", uuid::Uuid::new_v4().simple()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn create_append_load_and_branch() {
        let dir = temp();
        let store = Store::open(&dir).unwrap();
        let h = store.create("Hello", None, "single").unwrap();
        let u1 = Message::user("one");
        let a1 = Message::assistant("two").with_parent(Some(&u1.id));
        let u2 = Message::user("three").with_parent(Some(&a1.id));
        // A branch: another answer to the same user message.
        let a1b = Message::assistant("two, again").with_parent(Some(&u1.id));
        for m in [&u1, &a1, &u2, &a1b] {
            store.append(&h.id, m).unwrap();
        }
        let s = store.load(&h.id).unwrap();
        assert_eq!(s.messages.len(), 4);
        assert_eq!(s.children_of(&u1.id).len(), 2);
        let path: Vec<&str> = s
            .path_to(&u2.id)
            .iter()
            .map(|m| m.content.as_str())
            .collect();
        assert_eq!(path, vec!["one", "two", "three"]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn damaged_lines_are_skipped() {
        let dir = temp();
        let store = Store::open(&dir).unwrap();
        let h = store.create("T", None, "single").unwrap();
        store.append(&h.id, &Message::user("ok")).unwrap();
        let path = dir.join(format!("{}.jsonl", h.id));
        let mut f = OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(b"{this is not json\n").unwrap();
        store.append(&h.id, &Message::user("still ok")).unwrap();
        let s = store.load(&h.id).unwrap();
        assert_eq!((s.messages.len(), s.skipped), (2, 1));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn meta_updates_title_and_list_orders_newest_first() {
        let dir = temp();
        let store = Store::open(&dir).unwrap();
        let a = store.create("A", None, "single").unwrap();
        let b = store.create("B", None, "team").unwrap();
        let mut m = Message::user("later");
        m.ts += 10_000;
        store.append(&a.id, &m).unwrap();
        store
            .set_meta(&a.id, Some("Renamed".into()), None, None)
            .unwrap();
        let list = store.list().unwrap();
        assert_eq!(list[0].id, a.id);
        assert_eq!(list[0].title, "Renamed");
        assert_eq!(store.load(&b.id).unwrap().header.mode, "team");
        store.delete(&b.id).unwrap();
        assert!(store.load(&b.id).is_err());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn bad_ids_are_rejected() {
        let dir = temp();
        let store = Store::open(&dir).unwrap();
        assert!(store.load("../evil").is_err());
        assert!(store.load("").is_err());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn cycles_do_not_hang() {
        let mut a = Message::user("a");
        let mut b = Message::user("b");
        a.parent = Some(b.id.clone());
        b.parent = Some(a.id.clone());
        let s = Session {
            header: Header {
                id: "x".into(),
                title: "t".into(),
                created: 0,
                model: None,
                mode: "single".into(),
            },
            messages: vec![a.clone(), b],
            skipped: 0,
        };
        assert_eq!(s.path_to(&a.id).len(), 2);
    }
}

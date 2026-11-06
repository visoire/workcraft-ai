//! Undo for the files an answer changed. Before a tool writes a file, the file as it was (or the fact
//! that it did not exist) is saved under `<data>/checkpoints/<session>/<turn>/`. Restoring a turn puts
//! every file back, newest turn first if several are restored.
//!
//! A turn is the user message the answer replies to, so "undo this reply" is one call.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use workcraft_core::{Error, Result, now_ms};

/// Files bigger than this are not copied (the change is still listed, without undo).
const MAX_BACKUP_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    pub path: PathBuf,
    /// The file existed before the change.
    pub existed: bool,
    /// Copy of the old file inside the turn folder; `None` when it did not exist or was too big.
    pub backup: Option<String>,
    pub tool: String,
    pub ts: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Manifest {
    pub entries: Vec<Entry>,
    #[serde(default)]
    pub restored: bool,
}

pub struct Checkpoints {
    dir: PathBuf,
    lock: Mutex<()>,
}

fn safe(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

impl Checkpoints {
    #[must_use]
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            lock: Mutex::new(()),
        }
    }

    fn turn_dir(&self, session: &str, turn: &str) -> PathBuf {
        self.dir.join(safe(session)).join(safe(turn))
    }

    fn read_manifest(dir: &Path) -> Manifest {
        std::fs::read_to_string(dir.join("manifest.json"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    fn write_manifest(dir: &Path, m: &Manifest) -> Result<()> {
        std::fs::create_dir_all(dir)?;
        std::fs::write(dir.join("manifest.json"), serde_json::to_string_pretty(m)?)?;
        Ok(())
    }

    /// Remember `path` as it is now, once per turn (the first state is the one to go back to).
    pub fn save(&self, session: &str, turn: &str, tool: &str, path: &Path) -> Result<()> {
        if turn.is_empty() {
            return Ok(());
        }
        let _g = self
            .lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = self.turn_dir(session, turn);
        let mut m = Self::read_manifest(&dir);
        if m.entries.iter().any(|e| e.path == path) {
            return Ok(());
        }
        let existed = path.is_file();
        let mut backup = None;
        if existed && std::fs::metadata(path).map_or(0, |m| m.len()) <= MAX_BACKUP_BYTES {
            std::fs::create_dir_all(&dir)?;
            let name = format!("{}.bak", m.entries.len());
            std::fs::copy(path, dir.join(&name))?;
            backup = Some(name);
        }
        m.entries.push(Entry {
            path: path.to_path_buf(),
            existed,
            backup,
            tool: tool.to_string(),
            ts: now_ms(),
        });
        m.restored = false;
        Self::write_manifest(&dir, &m)
    }

    /// Every turn of a session that changed files, newest first.
    #[must_use]
    pub fn list(&self, session: &str) -> Value {
        let base = self.dir.join(safe(session));
        let mut turns: Vec<(i64, Value)> = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&base) {
            for e in rd.filter_map(std::result::Result::ok) {
                let m = Self::read_manifest(&e.path());
                if m.entries.is_empty() {
                    continue;
                }
                let ts = m.entries.iter().map(|x| x.ts).max().unwrap_or(0);
                turns.push((
                    ts,
                    json!({
                        "turn": e.file_name().to_string_lossy(),
                        "restored": m.restored,
                        "ts": ts,
                        "files": m.entries.iter().map(|x| json!({
                            "path": x.path.display().to_string(),
                            "created": !x.existed,
                            "can_undo": !x.existed || x.backup.is_some(),
                            "tool": x.tool,
                        })).collect::<Vec<_>>(),
                    }),
                ));
            }
        }
        turns.sort_by_key(|t| std::cmp::Reverse(t.0));
        json!(turns.into_iter().map(|t| t.1).collect::<Vec<_>>())
    }

    /// Put every file a turn changed back the way it was. Created files are removed.
    pub fn restore(&self, session: &str, turn: &str) -> Result<Value> {
        let _g = self
            .lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = self.turn_dir(session, turn);
        let mut m = Self::read_manifest(&dir);
        if m.entries.is_empty() {
            return Err(Error::NotFound(format!(
                "no changes recorded for turn '{turn}'"
            )));
        }
        let mut done = Vec::new();
        let mut failed = Vec::new();
        // Newest change first, so a file written twice ends at its first state.
        for e in m.entries.iter().rev() {
            let r = match (&e.existed, &e.backup) {
                (false, _) => {
                    if e.path.exists() {
                        std::fs::remove_file(&e.path)
                    } else {
                        Ok(())
                    }
                }
                (true, Some(b)) => {
                    if let Some(parent) = e.path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    std::fs::copy(dir.join(b), &e.path).map(|_| ())
                }
                (true, None) => Err(std::io::Error::other("the file was too big to keep a copy")),
            };
            match r {
                Ok(()) => done.push(e.path.display().to_string()),
                Err(err) => failed.push(format!("{}: {err}", e.path.display())),
            }
        }
        m.restored = true;
        Self::write_manifest(&dir, &m)?;
        Ok(json!({ "restored": done, "failed": failed }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restore_puts_files_back_and_removes_new_ones() {
        let root = std::env::temp_dir().join(format!("wc-ckpt-{}", uuid::Uuid::new_v4().simple()));
        let work = root.join("work");
        std::fs::create_dir_all(&work).unwrap();
        let cp = Checkpoints::new(root.join("checkpoints"));
        let old = work.join("old.txt");
        let new = work.join("new.txt");
        std::fs::write(&old, "before").unwrap();

        cp.save("s1", "t1", "edit_file", &old).unwrap();
        std::fs::write(&old, "after").unwrap();
        cp.save("s1", "t1", "write_file", &new).unwrap();
        std::fs::write(&new, "fresh").unwrap();
        // A second write in the same turn keeps the first state.
        cp.save("s1", "t1", "edit_file", &old).unwrap();
        std::fs::write(&old, "after again").unwrap();

        let l = cp.list("s1");
        assert_eq!(l[0]["files"].as_array().unwrap().len(), 2);
        let r = cp.restore("s1", "t1").unwrap();
        assert_eq!(r["failed"].as_array().unwrap().len(), 0);
        assert_eq!(std::fs::read_to_string(&old).unwrap(), "before");
        assert!(!new.exists());
        assert_eq!(cp.list("s1")[0]["restored"], true);
        assert!(cp.restore("s1", "ghost").is_err());
        let _ = std::fs::remove_dir_all(root);
    }
}

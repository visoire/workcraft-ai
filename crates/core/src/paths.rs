use std::io;
use std::path::{Path, PathBuf};

/// The one folder both WorkCraft programs share: `~/workcraft`. The workspace keeps its projects, `config.json` and trash
/// there; the chat keeps its own data in the `chat` folder inside it ([`data_dir`]).
pub const SHARED_FOLDER: &str = "workcraft";

/// Earlier names of that folder, newest first. `~/WorkCraft` is the same folder as `~/workcraft` on Windows and macOS.
pub const FORMER_FOLDERS: [&str; 3] = ["Workspace", "WorkCraft", "OpenCraft"];

/// The folder WorkCraft chat keeps its data in: sessions, `mcp.json`, `agents/`, `skills/`, `config.json`.
///
/// `WORKCRAFT_CHAT_HOME` wins. Otherwise `~/workcraft/chat`, inside the folder the workspace app uses ([`shared_root`]).
#[must_use]
pub fn data_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("WORKCRAFT_CHAT_HOME")
        && !p.is_empty()
    {
        return PathBuf::from(p);
    }
    let target = shared_root().join("chat");
    static MIGRATED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    MIGRATED.get_or_init(|| {
        if let Some(from) = former_chat_dir(&home_dir(), &target, &|p| p.exists()) {
            let _ = move_dir(&from, &target);
        }
    });
    target
}

/// An earlier chat folder (`~/Workspace/chat`, `~/WorkCraft/chat`, `~/OpenCraft/chat`) whose chats belong in `target`:
/// the first one that exists, when `target` does not exist yet.
#[must_use]
pub fn former_chat_dir(
    home: &Path,
    target: &Path,
    exists: &dyn Fn(&Path) -> bool,
) -> Option<PathBuf> {
    if exists(target) {
        return None;
    }
    FORMER_FOLDERS
        .iter()
        .map(|name| home.join(name).join("chat"))
        .find(|p| p != target && exists(p))
}

/// Moves a folder; when it cannot be renamed (another program holds it open, or it is on another disk) it is copied and
/// the original is left alone, so nothing is ever lost.
pub fn move_dir(from: &Path, to: &Path) -> io::Result<()> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    copy_dir_all(from, to)
}

fn copy_dir_all(from: &Path, to: &Path) -> io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let dest = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_all(&entry.path(), &dest)?;
        } else {
            std::fs::copy(entry.path(), dest)?;
        }
    }
    Ok(())
}

/// `~/workcraft`, or `WORKCRAFT_HOME` when set (the same variable the workspace reads). An earlier `~/Workspace` (and
/// before that `~/WorkCraft`, `~/OpenCraft`) is renamed to `~/workcraft` once; when it cannot be renamed (another program holds
/// it open) it is used where it is, so nothing is lost. The workspace applies the same rules (`src/server/home.ts`).
#[must_use]
pub fn shared_root() -> PathBuf {
    if let Some(p) = std::env::var_os("WORKCRAFT_HOME")
        && !p.is_empty()
    {
        return PathBuf::from(p);
    }
    choose_root(&home_dir(), &|p| p.exists(), &mut |from, to| {
        std::fs::rename(from, to)
    })
}

/// The choice of [`shared_root`] without touching the real file system, so it can be tested.
pub fn choose_root(
    home: &Path,
    exists: &dyn Fn(&Path) -> bool,
    rename: &mut dyn FnMut(&Path, &Path) -> io::Result<()>,
) -> PathBuf {
    let current = home.join(SHARED_FOLDER);
    if exists(&current) {
        return current;
    }
    for name in FORMER_FOLDERS {
        let former = home.join(name);
        if former == current || !exists(&former) {
            continue;
        }
        return match rename(&former, &current) {
            Ok(()) => current,
            Err(_) => former,
        };
    }
    current
}

#[must_use]
pub fn home_dir() -> PathBuf {
    for var in ["USERPROFILE", "HOME"] {
        if let Some(p) = std::env::var_os(var)
            && !p.is_empty()
        {
            return PathBuf::from(p);
        }
    }
    PathBuf::from(".")
}

#[must_use]
pub fn sessions_dir() -> PathBuf {
    data_dir().join("sessions")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashSet;

    fn present(names: &[&str]) -> HashSet<PathBuf> {
        names.iter().map(|n| Path::new("/home/u").join(n)).collect()
    }

    #[test]
    fn an_existing_shared_folder_is_used_as_it_is() {
        let have = present(&["workcraft", "Workspace"]);
        let renames = RefCell::new(Vec::new());
        let got = choose_root(Path::new("/home/u"), &|p| have.contains(p), &mut |a, b| {
            renames
                .borrow_mut()
                .push((a.to_path_buf(), b.to_path_buf()));
            Ok(())
        });
        assert_eq!(got, Path::new("/home/u/workcraft"));
        assert!(
            renames.borrow().is_empty(),
            "nothing is moved when the new folder already exists"
        );
    }

    #[test]
    fn the_earlier_workspace_folder_is_renamed_once() {
        let have = present(&["Workspace"]);
        let renames = RefCell::new(Vec::new());
        let got = choose_root(Path::new("/home/u"), &|p| have.contains(p), &mut |a, b| {
            renames
                .borrow_mut()
                .push((a.to_path_buf(), b.to_path_buf()));
            Ok(())
        });
        assert_eq!(got, Path::new("/home/u/workcraft"));
        assert_eq!(
            renames.borrow().as_slice(),
            [(
                PathBuf::from("/home/u/Workspace"),
                PathBuf::from("/home/u/workcraft")
            )]
        );
    }

    #[test]
    fn a_folder_that_cannot_be_renamed_is_used_in_place() {
        let have = present(&["Workspace"]);
        let got = choose_root(Path::new("/home/u"), &|p| have.contains(p), &mut |_, _| {
            Err(io::Error::new(io::ErrorKind::PermissionDenied, "in use"))
        });
        assert_eq!(
            got,
            Path::new("/home/u/Workspace"),
            "nothing is lost when another program holds the folder"
        );
    }

    #[test]
    fn chats_of_an_earlier_folder_name_move_to_the_new_one() {
        let have = present(&["workcraft", "Workspace", "Workspace/chat"]);
        let target = Path::new("/home/u/workcraft/chat");
        let got = former_chat_dir(Path::new("/home/u"), target, &|p| have.contains(p));
        assert_eq!(got, Some(PathBuf::from("/home/u/Workspace/chat")));
        // nothing to do once the new folder has chats, or when there is nothing old
        let have2 = present(&["workcraft/chat", "Workspace/chat"]);
        assert_eq!(
            former_chat_dir(Path::new("/home/u"), target, &|p| have2.contains(p)),
            None
        );
        let have3 = present(&["workcraft"]);
        assert_eq!(
            former_chat_dir(Path::new("/home/u"), target, &|p| have3.contains(p)),
            None
        );
    }

    #[test]
    fn move_dir_keeps_every_file() {
        let base = std::env::temp_dir().join(format!("wc-move-{}", std::process::id()));
        let from = base.join("old");
        std::fs::create_dir_all(from.join("sessions")).unwrap();
        std::fs::write(from.join("sessions").join("a.jsonl"), "{}").unwrap();
        std::fs::write(from.join("config.json"), "{}").unwrap();
        let to = base.join("new").join("chat");
        move_dir(&from, &to).unwrap();
        assert!(to.join("sessions").join("a.jsonl").is_file());
        assert!(to.join("config.json").is_file());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn a_fresh_home_gets_the_new_name() {
        let got = choose_root(Path::new("/home/u"), &|_| false, &mut |_, _| Ok(()));
        assert_eq!(got, Path::new("/home/u/workcraft"));
    }

    #[test]
    fn the_chat_lives_inside_the_shared_folder() {
        // Without the override variables the chat folder is a child of the shared root
        if std::env::var_os("WORKCRAFT_CHAT_HOME").is_none()
            && std::env::var_os("WORKCRAFT_HOME").is_none()
        {
            assert_eq!(data_dir(), shared_root().join("chat"));
        }
    }
}

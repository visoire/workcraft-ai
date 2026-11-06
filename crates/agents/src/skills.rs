//! Skills in the agentskills.io format: a folder with `SKILL.md` (YAML front matter with `name` and
//! `description`, then Markdown) and optional `references/`. The 17 bundled Craft app skills use this format.
//!
//! Skills are not pasted into the prompt. The prompt carries a one-line index and the model reads a skill
//! with the `read_skill` tool when it needs it.

use std::path::{Path, PathBuf};

use workcraft_core::{Error, Result, truncate_bytes};

const MAX_SKILL_BYTES: usize = 48 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub dir: PathBuf,
}

/// Parse the front matter of a `SKILL.md`: returns `(name, description, body)`.
/// Handles `key: value`, and folded (`>`) or literal (`|`) multi-line values.
#[must_use]
pub fn parse_front_matter(text: &str) -> (Option<String>, Option<String>, String) {
    let text = text.trim_start_matches('\u{feff}');
    let Some(rest) = text.strip_prefix("---") else {
        return (None, None, text.to_string());
    };
    let rest = rest.trim_start_matches(['\r', '\n']);
    let Some(end) = rest.find("\n---") else {
        return (None, None, text.to_string());
    };
    let (fm, body) = rest.split_at(end);
    let body = body
        .trim_start_matches("\n---")
        .trim_start_matches(['\r', '\n'])
        .to_string();

    let mut name = None;
    let mut description = None;
    let lines: Vec<&str> = fm.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        i += 1;
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        if line.starts_with(' ') || line.starts_with('\t') {
            continue;
        }
        let key = key.trim();
        let value = value.trim();
        let parsed = if matches!(value, ">" | "|" | ">-" | "|-" | ">+" | "|+") {
            let mut parts = Vec::new();
            while i < lines.len()
                && (lines[i].starts_with(' ')
                    || lines[i].starts_with('\t')
                    || lines[i].trim().is_empty())
            {
                parts.push(lines[i].trim());
                i += 1;
            }
            parts
                .iter()
                .filter(|p| !p.is_empty())
                .copied()
                .collect::<Vec<_>>()
                .join(" ")
        } else {
            value.trim_matches(['"', '\'']).to_string()
        };
        match key {
            "name" => name = Some(parsed),
            "description" => description = Some(parsed),
            _ => {}
        }
    }
    (name, description, body)
}

#[derive(Debug, Clone, Default)]
pub struct Skills {
    skills: Vec<Skill>,
}

impl Skills {
    /// Scan folders for skills. A name found in an earlier folder wins.
    #[must_use]
    pub fn scan(dirs: &[PathBuf]) -> Self {
        let mut skills: Vec<Skill> = Vec::new();
        for dir in dirs {
            let Ok(entries) = std::fs::read_dir(dir) else {
                continue;
            };
            let mut paths: Vec<PathBuf> =
                entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
            paths.sort();
            for p in paths {
                let file = p.join("SKILL.md");
                let Ok(text) = std::fs::read_to_string(&file) else {
                    continue;
                };
                let (name, description, _) = parse_front_matter(&text);
                let name = name
                    .filter(|n| !n.is_empty())
                    .or_else(|| p.file_name().map(|n| n.to_string_lossy().into_owned()));
                let Some(name) = name else { continue };
                if skills.iter().any(|s| s.name == name) {
                    continue;
                }
                skills.push(Skill {
                    name,
                    description: description.unwrap_or_default(),
                    dir: p,
                });
            }
        }
        Self { skills }
    }

    #[must_use]
    pub fn list(&self) -> &[Skill] {
        &self.skills
    }

    /// The index for the system prompt: one line per skill.
    #[must_use]
    pub fn index(&self, only: &[String]) -> String {
        let mut out = String::new();
        for s in self
            .skills
            .iter()
            .filter(|s| only.is_empty() || only.contains(&s.name))
        {
            let desc: String = s.description.chars().take(160).collect();
            out.push_str(&format!("- {}: {}\n", s.name, desc));
        }
        out
    }

    /// Like [`Skills::read`] for a model: a long skill comes as its first part plus the list of its sections,
    /// and `section` returns one section whole. A skill is 20 to 25 KB; read in full on every task it would be
    /// paid for on every round of the tool loop.
    pub fn read_view(
        &self,
        name: &str,
        file: Option<&str>,
        section: Option<&str>,
        full: bool,
    ) -> Result<String> {
        let whole = self.read(name, file)?;
        if let Some(want) = section.map(str::trim).filter(|s| !s.is_empty()) {
            return Ok(find_section(&whole, want).unwrap_or_else(|| {
                format!(
                    "No section matches \"{want}\". Sections:\n{}",
                    outline(&whole).join("\n")
                )
            }));
        }
        if full || whole.len() <= DIGEST_BYTES {
            return Ok(whole);
        }
        // Cut at the last heading before the limit so a section is never split in the middle.
        let mut cut = DIGEST_BYTES;
        while !whole.is_char_boundary(cut) {
            cut -= 1;
        }
        let at = whole[..cut].rfind("\n#").map_or(cut, |i| i + 1);
        let mut out = whole[..at].trim_end().to_string();
        let rest = outline_from(&whole, at);
        if !rest.is_empty() {
            out.push_str(&format!(
                "\n\n[Shortened: {} of {} KB shown. Read a part with read_skill section=\"<title>\" or everything with full=true. Remaining sections:\n{}]",
                at / 1024,
                whole.len() / 1024,
                rest.join("\n")
            ));
        }
        Ok(out)
    }

    /// Body of `SKILL.md`, or of a file under the skill folder (`references/commands.md`).
    pub fn read(&self, name: &str, file: Option<&str>) -> Result<String> {
        // Models write "WordCraft" as often as "wordcraft": match without case.
        let want = name.trim().to_lowercase();
        let s = self
            .skills
            .iter()
            .find(|s| s.name.to_lowercase() == want)
            .ok_or_else(|| {
                let names: Vec<&str> = self.skills.iter().map(|s| s.name.as_str()).collect();
                Error::NotFound(if names.is_empty() {
                    format!("skill '{name}': no skills are installed")
                } else {
                    format!("skill '{name}'. Available: {}", names.join(", "))
                })
            })?;
        let path = match file {
            None => s.dir.join("SKILL.md"),
            Some(f) => safe_join(&s.dir, f)?,
        };
        let text = std::fs::read_to_string(&path)
            .map_err(|e| Error::Io(format!("{}: {e}", path.display())))?;
        let body = if file.is_none() {
            parse_front_matter(&text).2
        } else {
            text
        };
        let mut out = truncate_bytes(&body, MAX_SKILL_BYTES);
        if file.is_none()
            && let Ok(rd) = std::fs::read_dir(s.dir.join("references"))
        {
            let mut names: Vec<String> = rd
                .filter_map(|e| {
                    e.ok()
                        .map(|e| format!("references/{}", e.file_name().to_string_lossy()))
                })
                .collect();
            names.sort();
            if !names.is_empty() {
                out.push_str(&format!(
                    "\n\n[More files, read with read_skill file=...: {}]",
                    names.join(", ")
                ));
            }
        }
        Ok(out)
    }
}

/// Largest first part of a skill a model gets without asking for more.
const DIGEST_BYTES: usize = 7000;

/// Heading lines of a Markdown text as `- title (size KB)`, in order, `##` and deeper.
fn outline(text: &str) -> Vec<String> {
    outline_from(text, 0)
}

fn outline_from(text: &str, from: usize) -> Vec<String> {
    let mut heads: Vec<(usize, &str)> = Vec::new();
    let mut pos = 0usize;
    for line in text.split_inclusive('\n') {
        if pos >= from && line.starts_with("## ") {
            heads.push((pos, line.trim_start_matches('#').trim()));
        }
        pos += line.len();
    }
    heads
        .iter()
        .enumerate()
        .map(|(i, (start, title))| {
            let end = heads.get(i + 1).map_or(text.len(), |(s, _)| *s);
            format!("- {title} ({} KB)", (end - start).div_ceil(1024))
        })
        .collect()
}

/// The part of `text` from the first heading containing `want` (any case) to the next heading of the same
/// or a higher level.
fn find_section(text: &str, want: &str) -> Option<String> {
    let want = want.to_lowercase();
    let mut start: Option<(usize, usize)> = None; // (byte offset, heading level)
    let mut pos = 0usize;
    for line in text.split_inclusive('\n') {
        let level = line.chars().take_while(|c| *c == '#').count();
        if level > 0 && line[level..].starts_with(' ') {
            match start {
                None if line.to_lowercase().contains(&want) => start = Some((pos, level)),
                Some((s, lv)) if level <= lv => return Some(text[s..pos].trim_end().to_string()),
                _ => {}
            }
        }
        pos += line.len();
    }
    start.map(|(s, _)| text[s..].trim_end().to_string())
}
mod bundled {
    include!(concat!(env!("OUT_DIR"), "/bundled_skills.rs"));
}

/// Write the skills that ship with WorkCraft (the 17 Craft app skills) into `dir`. A skill folder that
/// already exists is left alone, so edits the user made survive updates. Returns the names installed.
pub fn install_bundled(dir: &Path) -> Vec<String> {
    let mut installed = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    for (rel, bytes) in bundled::BUNDLED {
        let Some((skill, _)) = rel.split_once('/') else {
            continue;
        };
        if skipped.iter().any(|s| s == skill) {
            continue;
        }
        let folder = dir.join(skill);
        if folder.exists() && !installed.iter().any(|s| s == skill) {
            skipped.push(skill.to_string());
            continue;
        }
        let path = dir.join(rel);
        if let Some(parent) = path.parent()
            && std::fs::create_dir_all(parent).is_err()
        {
            continue;
        }
        if std::fs::write(&path, bytes).is_ok() && !installed.iter().any(|s| s == skill) {
            installed.push(skill.to_string());
        }
    }
    installed
}

/// Join a relative path under `base` without letting it climb out.
fn safe_join(base: &Path, rel: &str) -> Result<PathBuf> {
    let rel_path = Path::new(rel);
    if rel_path.is_absolute()
        || rel_path.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir | std::path::Component::Prefix(_)
            )
        })
    {
        return Err(Error::Denied(format!(
            "'{rel}' is outside the skill folder"
        )));
    }
    Ok(base.join(rel_path))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FOLDED: &str = "---\nname: photocraft\ndescription: >\n  PhotoCraft is a native,\n  pure-Rust image editor.\n---\n\n# PhotoCraft\n\nBody.\n";

    #[test]
    fn parses_folded_and_plain_front_matter() {
        let (n, d, body) = parse_front_matter(FOLDED);
        assert_eq!(n.as_deref(), Some("photocraft"));
        assert_eq!(
            d.as_deref(),
            Some("PhotoCraft is a native, pure-Rust image editor.")
        );
        assert!(body.starts_with("# PhotoCraft"));
        let (n, d, _) = parse_front_matter("---\nname: \"x\"\ndescription: short one\n---\nbody");
        assert_eq!((n.as_deref(), d.as_deref()), (Some("x"), Some("short one")));
        let (n, _, body) = parse_front_matter("no front matter");
        assert_eq!((n, body.as_str()), (None, "no front matter"));
    }

    #[test]
    fn scans_reads_and_confines_paths() {
        let root =
            std::env::temp_dir().join(format!("wc-skills-{}", uuid::Uuid::new_v4().simple()));
        let skill = root.join("photocraft");
        std::fs::create_dir_all(skill.join("references")).unwrap();
        std::fs::write(skill.join("SKILL.md"), FOLDED).unwrap();
        std::fs::write(skill.join("references").join("cmds.md"), "commands").unwrap();
        std::fs::create_dir_all(root.join("empty")).unwrap();

        let s = Skills::scan(std::slice::from_ref(&root));
        assert_eq!(s.list().len(), 1);
        assert!(s.index(&[]).contains("photocraft: PhotoCraft is a native"));
        assert!(s.index(&["other".to_string()]).is_empty());
        let body = s.read("photocraft", None).unwrap();
        assert!(body.contains("Body.") && body.contains("references/cmds.md"));
        assert_eq!(
            s.read("photocraft", Some("references/cmds.md")).unwrap(),
            "commands"
        );
        assert!(s.read("photocraft", Some("../secret")).is_err());
        assert!(
            s.read("PhotoCraft", None).is_ok(),
            "names match without case"
        );
        let e = s.read("nope", None).unwrap_err().to_string();
        assert!(e.contains("Available: photocraft"), "{e}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn bundled_skills_install_once_and_keep_user_edits() {
        let root =
            std::env::temp_dir().join(format!("wc-bundle-{}", uuid::Uuid::new_v4().simple()));
        let first = install_bundled(&root);
        assert_eq!(first.len(), 17, "{first:?}");
        let s = Skills::scan(std::slice::from_ref(&root));
        assert_eq!(s.list().len(), 17);
        assert!(s.read("WordCraft", None).unwrap().contains("wordcraft-cli"));
        // A user edit survives a second install.
        let f = root.join("wordcraft").join("SKILL.md");
        std::fs::write(&f, "---\nname: wordcraft\ndescription: mine\n---\nmine").unwrap();
        assert!(install_bundled(&root).is_empty());
        assert!(std::fs::read_to_string(&f).unwrap().contains("mine"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_real_artcraft_skills_parse_when_present() {
        let dir = PathBuf::from(r"C:\xampp\htdocs\artcraft\artcraft\artcraft-skills");
        if !dir.exists() {
            return;
        }
        let s = Skills::scan(&[dir]);
        assert_eq!(s.list().len(), 13);
        assert!(
            s.list().iter().all(|k| !k.description.is_empty()),
            "every skill has a description"
        );
    }

    #[test]
    fn sections_are_listed_and_found_whole() {
        let text = "# T\nintro\n## One\na\n### One.a\nb\n## Two\nc\n";
        assert_eq!(outline(text), ["- One (1 KB)", "- Two (1 KB)"]);
        // A section runs to the next heading of the same or a higher level, subsections included.
        assert_eq!(
            find_section(text, "one").unwrap(),
            "## One\na\n### One.a\nb"
        );
        assert_eq!(find_section(text, "ONE.A").unwrap(), "### One.a\nb");
        assert_eq!(find_section(text, "two").unwrap(), "## Two\nc");
        assert!(find_section(text, "three").is_none());
    }
}

//! Craft application discovery across installer layouts, `<APP>_CLI` overrides, and `PATH` (section 5.2).

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use crate::craft::live::live_port_for;
use crate::craft::spec::{AppSpec, SPECS, find_spec};

#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub id: &'static str,
    pub name: &'static str,
    pub kind: &'static str,
    pub cli: PathBuf,
    pub gui: Option<PathBuf>,
    pub version: Option<String>,
    /// Port of a running app that accepts a live bridge (`--connect` / `--bridge` / `--attach`).
    pub live_port: Option<u16>,
}

#[must_use]
pub fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

/// Default Windows/macOS/Linux installation base directories on this computer.
#[must_use]
pub fn default_install_bases() -> Vec<PathBuf> {
    let mut bases = Vec::new();
    if cfg!(windows) {
        for var in [
            "ProgramFiles",
            "ProgramW6432",
            "ProgramFiles(x86)",
            "LOCALAPPDATA",
        ] {
            if let Some(val) = std::env::var_os(var) {
                let p = PathBuf::from(val);
                if !bases.contains(&p) {
                    bases.push(p.clone());
                }
                let progs = p.join("Programs");
                if !bases.contains(&progs) {
                    bases.push(progs);
                }
            }
        }
        // `%USERPROFILE%\ArtCraft\<App>` (a per-user install next to the launcher)
        let home = workcraft_core::paths::home_dir();
        if !bases.contains(&home) {
            bases.push(home);
        }
    } else if cfg!(target_os = "macos") {
        bases.push(PathBuf::from("/Applications"));
        let user_apps = workcraft_core::paths::home_dir().join("Applications");
        bases.push(user_apps);
    } else {
        bases.push(PathBuf::from("/opt"));
        bases.push(PathBuf::from("/usr/local"));
    }
    bases
}

/// Candidate executable paths for `bin_stem` of `spec` under `install_bases` plus `PATH`.
#[must_use]
pub fn candidates_in_bases(
    spec: &AppSpec,
    bin_stem: &str,
    install_bases: &[PathBuf],
    include_path: bool,
) -> Vec<PathBuf> {
    let bin = exe(bin_stem);
    let mut out = Vec::new();
    for base in install_bases {
        if cfg!(target_os = "macos") {
            out.push(
                base.join(format!("{}.app", spec.name))
                    .join("Contents/MacOS")
                    .join(&bin),
            );
        }
        for rel in spec.install_dirs {
            let mut sub = base.clone();
            for seg in rel.split('/') {
                sub.push(seg);
            }
            out.push(sub.join(&bin));
        }
    }
    if !cfg!(windows) {
        for dir in ["/usr/local/bin", "/opt/homebrew/bin", "/usr/bin"] {
            out.push(PathBuf::from(dir).join(&bin));
        }
        out.push(
            workcraft_core::paths::home_dir()
                .join(".local/bin")
                .join(&bin),
        );
    }
    if include_path && let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            if dir.is_absolute() {
                out.push(dir.join(&bin));
            }
        }
    }
    out
}

/// Other file names an app's command-line tool has had (SolidCraft's came first as `solvecraft-cli`).
#[must_use]
pub fn cli_stems(spec: &AppSpec) -> Vec<&'static str> {
    let mut stems = vec![spec.cli];
    if spec.id == "solidcraft" {
        stems.push("solvecraft-cli");
    }
    stems
}

/// Looks one level into every folder of the install bases whose name belongs to the app or to ArtCraft
/// (`Program Files\Art Craft\WordCraft 0.15`, `ArtCraft Apps\wordcraft`), for installers that do not use the standard names.
fn scan_install_bases(
    spec: &AppSpec,
    stems: &[&str],
    install_bases: &[PathBuf],
) -> Option<PathBuf> {
    let id = spec.id.to_ascii_lowercase();
    let name = spec.name.to_ascii_lowercase();
    let squash = |s: &str| s.to_ascii_lowercase().replace([' ', '-', '_'], "");
    for base in install_bases {
        let Ok(entries) = std::fs::read_dir(base) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(ft) = entry.file_type() else { continue };
            if !ft.is_dir() {
                continue;
            }
            let dir_name = squash(&entry.file_name().to_string_lossy());
            let own = dir_name.contains(&id) || dir_name.contains(&squash(&name));
            let suite = dir_name.starts_with("artcraft");
            if !own && !suite {
                continue;
            }
            for stem in stems {
                let bin = exe(stem);
                let direct = entry.path().join(&bin);
                if own && direct.is_file() {
                    return Some(direct);
                }
                if suite && let Ok(inner) = std::fs::read_dir(entry.path()) {
                    for sub in inner.flatten() {
                        let sub_name = squash(&sub.file_name().to_string_lossy());
                        if (sub_name.contains(&id) || sub_name.contains(&squash(&name)))
                            && sub.path().join(&bin).is_file()
                        {
                            return Some(sub.path().join(&bin));
                        }
                    }
                }
            }
        }
    }
    None
}

/// Locate the desktop GUI executable for `spec`, preferring the sibling of `cli`.
#[must_use]
pub fn find_gui_for_spec(
    spec: &AppSpec,
    cli: Option<&Path>,
    install_bases: &[PathBuf],
) -> Option<PathBuf> {
    if let Some(c) = cli
        && let Some(parent) = c.parent()
    {
        let sibling = parent.join(exe(spec.gui));
        if sibling.is_file() {
            return Some(sibling);
        }
    }
    candidates_in_bases(spec, spec.gui, install_bases, true)
        .into_iter()
        .find(|p| p.is_file())
}

/// Backwards-compatible GUI lookup by app id and display name.
#[must_use]
pub fn find_gui(id: &str, _name: &str, cli: Option<&Path>) -> Option<PathBuf> {
    let spec = find_spec(id)?;
    find_gui_for_spec(spec, cli, &default_install_bases())
}

type VersionCache = Mutex<HashMap<(PathBuf, Option<SystemTime>), Option<String>>>;

fn version_cache() -> &'static VersionCache {
    static CACHE: OnceLock<VersionCache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Probe `<cli> --version` once and cache the result by executable path and mtime.
#[must_use]
pub fn probe_version(cli: &Path) -> Option<String> {
    let mtime = std::fs::metadata(cli).ok().and_then(|m| m.modified().ok());
    let key = (cli.to_path_buf(), mtime);
    if let Some(cached) = version_cache()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&key)
        .cloned()
    {
        return cached;
    }

    let mut cmd = std::process::Command::new(cli);
    cmd.arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let ver = cmd.output().ok().and_then(|out| {
        let s = String::from_utf8_lossy(&out.stdout);
        let fallback = String::from_utf8_lossy(&out.stderr);
        let line = s
            .lines()
            .find(|l| !l.trim().is_empty())
            .or_else(|| fallback.lines().find(|l| !l.trim().is_empty()))?
            .trim();
        // Extract trailing semver token if `<cli> 0.20.2` is printed, else keep the short line.
        let token = line
            .split_whitespace()
            .find(|part| part.chars().next().is_some_and(|c| c.is_ascii_digit()))
            .unwrap_or(line);
        Some(token.to_string())
    });

    version_cache()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(key, ver.clone());
    ver
}

/// Discover installed Craft apps in `install_bases`, returning both `Found` entries and explicit
/// `<APP>_CLI` override errors (no silent fallback when an override variable is broken).
#[must_use]
pub fn discover_in_bases(
    install_bases: &[PathBuf],
    include_path: bool,
    probe_versions: bool,
) -> (Vec<Found>, BTreeMap<&'static str, String>) {
    if std::env::var_os("WORKCRAFT_NO_CRAFT_DISCOVERY").is_some_and(|v| !v.is_empty()) {
        return (Vec::new(), BTreeMap::new());
    }
    let mut found = Vec::new();
    let mut errors = BTreeMap::new();

    for spec in &SPECS {
        if let Some(raw_override) = std::env::var_os(spec.env_override) {
            let trimmed = raw_override.to_string_lossy().trim().to_string();
            if !trimmed.is_empty() {
                let p = PathBuf::from(&trimmed);
                if p.is_file() {
                    let gui = find_gui_for_spec(spec, Some(&p), install_bases);
                    let version = probe_versions.then(|| probe_version(&p)).flatten();
                    found.push(Found {
                        id: spec.id,
                        name: spec.name,
                        kind: spec.kind,
                        cli: p,
                        gui,
                        version,
                        live_port: spec
                            .supports_live()
                            .then(|| live_port_for(spec.id, spec.name))
                            .flatten(),
                    });
                } else {
                    errors.insert(
                        spec.id,
                        format!(
                            "{} points to '{}', which does not exist",
                            spec.env_override, trimmed
                        ),
                    );
                }
                continue;
            }
        }

        let stems = cli_stems(spec);
        let Some(cli) = stems
            .iter()
            .flat_map(|stem| candidates_in_bases(spec, stem, install_bases, include_path))
            .find(|p| p.is_file())
            .or_else(|| scan_install_bases(spec, &stems, install_bases))
        else {
            continue;
        };
        let gui = find_gui_for_spec(spec, Some(&cli), install_bases);
        let version = probe_versions.then(|| probe_version(&cli)).flatten();
        found.push(Found {
            id: spec.id,
            name: spec.name,
            kind: spec.kind,
            cli,
            gui,
            version,
            live_port: spec
                .supports_live()
                .then(|| live_port_for(spec.id, spec.name))
                .flatten(),
        });
    }

    (found, errors)
}

/// Discover all installed Craft apps plus any `<APP>_CLI` override errors.
#[must_use]
pub fn discover_with_errors() -> (Vec<Found>, BTreeMap<&'static str, String>) {
    discover_in_bases(&default_install_bases(), true, true)
}

/// Every Craft app found on this computer. `WORKCRAFT_NO_CRAFT_DISCOVERY=1` turns this off (tests).
#[must_use]
pub fn discover() -> Vec<Found> {
    discover_with_errors().0
}

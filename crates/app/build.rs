//! Embed the Windows application icon and version metadata into `workcraft.exe`.

use std::path::{Path, PathBuf};
use std::process::Command;

fn find_rc() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("RC") {
        let pb = PathBuf::from(p);
        if pb.is_file() {
            return Some(pb);
        }
    }
    let kits = Path::new(r"C:\Program Files (x86)\Windows Kits\10\bin");
    let Ok(rd) = std::fs::read_dir(kits) else {
        return None;
    };
    let mut versions: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
    versions.sort();
    versions.reverse();
    for v in versions {
        let rc = v.join("x64").join("rc.exe");
        if rc.is_file() {
            return Some(rc);
        }
    }
    None
}

fn main() {
    println!("cargo:rerun-if-changed=assets/workcraft.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    let ico = manifest_dir.join("assets").join("workcraft.ico");
    if !ico.is_file() {
        return;
    }
    let Some(rc_exe) = find_rc() else {
        return;
    };
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap_or_default());
    let rc_path = out_dir.join("workcraft.rc");
    let res_path = out_dir.join("workcraft.res");
    let ico_escaped = ico.to_string_lossy().replace('\\', "\\\\");
    let rc_content = format!(
        "1 ICON \"{ico_escaped}\"\n\
         1 VERSIONINFO\n\
         FILEVERSION 0,1,0,0\n\
         PRODUCTVERSION 0,1,0,0\n\
         BEGIN\n\
           BLOCK \"StringFileInfo\"\n\
           BEGIN\n\
             BLOCK \"040904b0\"\n\
             BEGIN\n\
               VALUE \"CompanyName\", \"WorkCraft\"\n\
               VALUE \"FileDescription\", \"WorkCraft Desktop Assistant\"\n\
               VALUE \"FileVersion\", \"0.1.0-alpha.9\"\n\
               VALUE \"InternalName\", \"workcraft\"\n\
               VALUE \"OriginalFilename\", \"WorkCraft.exe\"\n\
               VALUE \"ProductName\", \"WorkCraft\"\n\
               VALUE \"ProductVersion\", \"0.1.0-alpha.9\"\n\
             END\n\
           END\n\
           BLOCK \"VarFileInfo\"\n\
           BEGIN\n\
             VALUE \"Translation\", 0x409, 1200\n\
           END\n\
         END\n"
    );
    if std::fs::write(&rc_path, rc_content).is_err() {
        return;
    }
    let status = Command::new(rc_exe)
        .arg("/nologo")
        .arg("/fo")
        .arg(&res_path)
        .arg(&rc_path)
        .status();
    if status.is_ok_and(|s| s.success()) && res_path.is_file() {
        println!("cargo:rustc-link-arg-bins={}", res_path.display());
    }
}

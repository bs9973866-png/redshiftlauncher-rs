//! Загрузка и поиск Java (Temurin JRE через Adoptium).

use crate::install::Dirs;
use crate::net::{self, Job, Result};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

pub fn adoptium_url(major: u32, windows: bool) -> String {
    let os = if windows { "windows" } else { "linux" };
    format!("https://api.adoptium.net/v3/binary/latest/{major}/ga/{os}/x64/jre/hotspot/normal/eclipse")
}

pub fn exe_name(windows: bool) -> &'static str { if windows { "java.exe" } else { "java" } }

/// Ищет bin/java[.exe] внутри распакованного JRE (папка вида jdk-17.0.9+9-jre).
pub fn find_java(root: &Path, windows: bool) -> Option<PathBuf> {
    let direct = root.join("bin").join(exe_name(windows));
    if direct.is_file() { return Some(direct); }
    for e in fs::read_dir(root).ok()?.flatten() {
        let p = e.path();
        if p.is_dir() { if let Some(j) = find_java(&p, windows) { return Some(j); } }
    }
    None
}

pub fn runtime_dir(dirs: &Dirs, major: u32) -> PathBuf { dirs.runtimes().join(format!("java{major}")) }

pub fn unzip(zip_path: &Path, dest: &Path) -> Result<()> {
    let f = fs::File::open(zip_path).map_err(|e| e.to_string())?;
    let mut z = zip::ZipArchive::new(f).map_err(|e| e.to_string())?;
    for i in 0..z.len() {
        let mut e = z.by_index(i).map_err(|e| e.to_string())?;
        let Some(rel) = e.enclosed_name() else { continue };
        let out = dest.join(rel);
        if e.is_dir() { fs::create_dir_all(&out).map_err(|e| e.to_string())?; continue; }
        if let Some(p) = out.parent() { fs::create_dir_all(p).map_err(|e| e.to_string())?; }
        let mut buf = vec![]; e.read_to_end(&mut buf).map_err(|e| e.to_string())?;
        fs::write(&out, &buf).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        if let Some(m) = e.unix_mode() { use std::os::unix::fs::PermissionsExt; let _ = fs::set_permissions(&out, fs::Permissions::from_mode(m)); }
    }
    Ok(())
}

/// Возвращает путь к java нужной версии, скачивая при необходимости.
/// Системная Java не используется намеренно — неподходящая версия была главной причиной сбоев.
pub fn ensure(agent: &ureq::Agent, dirs: &Dirs, major: u32, windows: bool) -> Result<PathBuf> {
    let dir = runtime_dir(dirs, major);
    if let Some(j) = find_java(&dir, windows) { return Ok(j); }
    ensure_from(agent, dirs, major, windows, &adoptium_url(major, windows))
}

pub fn ensure_from(agent: &ureq::Agent, dirs: &Dirs, major: u32, windows: bool, url: &str) -> Result<PathBuf> {
    let dir = runtime_dir(dirs, major);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let zip = dir.join("jre.zip");
    net::download_one(agent, &Job { url: url.into(), dest: zip.clone(), sha1: None })?;
    unzip(&zip, &dir)?;
    let _ = fs::remove_file(&zip);
    find_java(&dir, windows).ok_or_else(|| format!("в архиве Java {major} не найден {}", exe_name(windows)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::io::Write;

    #[test]
    fn urls() {
        assert!(adoptium_url(17, true).contains("/17/ga/windows/x64/jre/"));
        assert!(adoptium_url(8, false).contains("/8/ga/linux/"));
    }

    #[test]
    fn downloads_unpacks_and_finds_java() {
        let mut buf = std::io::Cursor::new(vec![]);
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            let o = zip::write::SimpleFileOptions::default();
            z.start_file("jdk-17.0.9+9-jre/bin/java.exe", o).unwrap(); z.write_all(b"MZ").unwrap();
            z.start_file("../evil.txt", o).unwrap(); z.write_all(b"x").unwrap();
            z.finish().unwrap();
        }
        let mut f = HashMap::new(); f.insert("/jre.zip".to_string(), buf.into_inner());
        let base = net::testsrv::serve(f);
        let root = std::env::temp_dir().join(format!("rs-jre-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let dirs = Dirs { root: root.clone() };
        let j = ensure_from(&net::agent(), &dirs, 17, true, &format!("{base}/jre.zip")).unwrap();
        assert!(j.ends_with("bin/java.exe"));
        assert!(!root.join("runtime/evil.txt").exists() && !root.join("evil.txt").exists());
        // повторный вызов находит без сети
        assert_eq!(ensure(&net::agent(), &dirs, 17, true).unwrap(), j);
    }
}

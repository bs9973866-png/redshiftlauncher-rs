//! Fabric / Quilt (профиль JSON) и Forge / NeoForge (запуск официального установщика).

use crate::install::Dirs;
use crate::net::{self, Job, Result};
use crate::runner;
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Loader { Vanilla, Fabric, Quilt, Forge, NeoForge }

impl Loader {
    pub fn parse(s: &str) -> Loader {
        match s { "fabric" => Loader::Fabric, "quilt" => Loader::Quilt, "forge" => Loader::Forge, "neoforge" => Loader::NeoForge, _ => Loader::Vanilla }
    }
    pub fn id(self) -> &'static str {
        match self { Loader::Vanilla => "vanilla", Loader::Fabric => "fabric", Loader::Quilt => "quilt", Loader::Forge => "forge", Loader::NeoForge => "neoforge" }
    }
}

// ---------- Fabric / Quilt ----------

pub fn meta_base(l: Loader) -> Option<&'static str> {
    match l { Loader::Fabric => Some("https://meta.fabricmc.net/v2"), Loader::Quilt => Some("https://meta.quiltmc.org/v3"), _ => None }
}

/// Первая стабильная версия загрузчика из ответа `/versions/loader/{mc}`.
pub fn pick_loader_version(list: &Value) -> Option<String> {
    let arr = list.as_array()?;
    let pick = |stable_only: bool| arr.iter().find(|e| {
        let l = e.get("loader").unwrap_or(e);
        !stable_only || l.get("stable").and_then(|s| s.as_bool()).unwrap_or(true) && !l.get("version").and_then(|v| v.as_str()).unwrap_or("").contains("beta")
    }).and_then(|e| e.get("loader").unwrap_or(e).get("version")?.as_str().map(String::from));
    pick(true).or_else(|| pick(false))
}

pub fn install_profile_loader(agent: &ureq::Agent, dirs: &Dirs, l: Loader, mc: &str, version: Option<&str>) -> Result<String> {
    let base = meta_base(l).ok_or("не Fabric/Quilt")?;
    let lv = match version {
        Some(v) => v.to_string(),
        None => pick_loader_version(&net::get_json(agent, &format!("{base}/versions/loader/{mc}"))?)
            .ok_or_else(|| format!("для {mc} нет версии {}", l.id()))?,
    };
    let prof = net::get_json(agent, &format!("{base}/versions/loader/{mc}/{lv}/profile/json"))?;
    let id = prof.get("id").and_then(|i| i.as_str()).map(String::from).unwrap_or_else(|| format!("{}-loader-{lv}-{mc}", l.id()));
    write_profile(dirs, &id, &prof)?;
    Ok(id)
}

pub fn write_profile(dirs: &Dirs, id: &str, prof: &Value) -> Result<()> {
    if id.contains(['/', '\\']) || id.contains("..") { return Err(format!("небезопасный id версии: {id}")); }
    fs::create_dir_all(dirs.version_dir(id)).map_err(|e| e.to_string())?;
    fs::write(dirs.version_json(id), serde_json::to_vec_pretty(prof).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

// ---------- Forge / NeoForge ----------

/// `promotions_slim.json` -> версия Forge для MC (recommended, иначе latest).
pub fn forge_version(promos: &Value, mc: &str) -> Option<String> {
    let p = promos.get("promos")?;
    p.get(format!("{mc}-recommended")).or_else(|| p.get(format!("{mc}-latest")))?.as_str().map(String::from)
}

pub fn forge_installer_url(mc: &str, fv: &str) -> String {
    format!("https://maven.minecraftforge.net/net/minecraftforge/forge/{mc}-{fv}/forge-{mc}-{fv}-installer.jar")
}

/// 1.21.1 -> префикс "21.1."; 1.20.1 -> "20.1."; для 1.21 -> "21.0."
pub fn neoforge_prefix(mc: &str) -> Option<String> {
    let mut it = mc.split('.');
    if it.next()? != "1" { return None; }
    let minor: u32 = it.next()?.parse().ok()?;
    let patch: u32 = it.next().map(|p| p.parse().unwrap_or(0)).unwrap_or(0);
    Some(format!("{minor}.{patch}."))
}

pub fn neoforge_version(versions: &[String], mc: &str) -> Option<String> {
    let pre = neoforge_prefix(mc)?;
    let key = |v: &str| -> Vec<u32> { v.split(|c: char| !c.is_ascii_digit()).filter_map(|x| x.parse().ok()).collect() };
    let mut c: Vec<&String> = versions.iter().filter(|v| v.starts_with(&pre)).collect();
    // стабильные (без -beta) приоритетнее
    c.sort_by(|a, b| (!a.contains("beta"), key(a)).cmp(&(!b.contains("beta"), key(b))));
    c.last().map(|s| s.to_string())
}

pub fn neoforge_installer_url(v: &str) -> String {
    format!("https://maven.neoforged.net/releases/net/neoforged/neoforge/{v}/neoforge-{v}-installer.jar")
}

pub fn list_versions(dirs: &Dirs) -> HashSet<String> {
    fs::read_dir(dirs.versions()).map(|d| d.flatten().filter(|e| e.path().is_dir()).filter_map(|e| e.file_name().into_string().ok()).collect()).unwrap_or_default()
}

/// Запускает установщик в headless-режиме. Возвращает id появившейся версии.
pub fn run_installer(java: &Path, installer: &Path, dirs: &Dirs, on_line: &(dyn Fn(&str) + Sync)) -> Result<String> {
    // установщик требует launcher_profiles.json
    let lp = dirs.root.join("launcher_profiles.json");
    if !lp.exists() { fs::write(&lp, r#"{"profiles":{},"version":3}"#).map_err(|e| e.to_string())?; }
    let before = list_versions(dirs);
    let out = runner::run(&java.to_string_lossy(),
        &["-jar".into(), installer.to_string_lossy().into(), "--installClient".into(), dirs.root.to_string_lossy().into()], &dirs.root, on_line)?;
    if out.code != Some(0) {
        let tail: Vec<&str> = out.tail.lines().rev().take(6).collect();
        return Err(format!("установщик завершился с кодом {:?}: {}", out.code, tail.into_iter().rev().collect::<Vec<_>>().join(" | ")));
    }
    let after = list_versions(dirs);
    let mut new: Vec<String> = after.difference(&before).cloned().collect();
    new.sort();
    new.pop().ok_or_else(|| "установщик отработал, но новая версия не появилась (возможно, она уже была установлена)".into())
}

pub fn find_existing(dirs: &Dirs, l: Loader, mc: &str) -> Option<String> {
    let needle = match l { Loader::Forge => "forge", Loader::NeoForge => "neoforge", _ => return None };
    let mut v: Vec<String> = list_versions(dirs).into_iter().filter(|v| v.to_lowercase().contains(needle) && v.contains(mc) || (l == Loader::NeoForge && v.to_lowercase().contains(needle))).collect();
    v.sort();
    v.pop()
}

pub fn install_installer_loader(agent: &ureq::Agent, dirs: &Dirs, java: &Path, l: Loader, mc: &str, tmp: &Path,
                                on_line: &(dyn Fn(&str) + Sync)) -> Result<String> {
    let url = match l {
        Loader::Forge => {
            let p = net::get_json(agent, "https://files.minecraftforge.net/net/minecraftforge/forge/promotions_slim.json")?;
            let fv = forge_version(&p, mc).ok_or_else(|| format!("для {mc} нет Forge"))?;
            forge_installer_url(mc, &fv)
        }
        Loader::NeoForge => {
            let j = net::get_json(agent, "https://maven.neoforged.net/api/maven/versions/releases/net/neoforged/neoforge")?;
            let all: Vec<String> = j.get("versions").and_then(|v| v.as_array()).into_iter().flatten().filter_map(|v| v.as_str().map(String::from)).collect();
            neoforge_installer_url(&neoforge_version(&all, mc).ok_or_else(|| format!("для {mc} нет NeoForge"))?)
        }
        _ => return Err("не Forge/NeoForge".into()),
    };
    fs::create_dir_all(tmp).map_err(|e| e.to_string())?;
    let jar: PathBuf = tmp.join("installer.jar");
    net::download_one(agent, &Job { url, dest: jar.clone(), sha1: None })?;
    let id = run_installer(java, &jar, dirs, on_line);
    let _ = fs::remove_file(&jar);
    id
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn picks_stable_fabric_loader() {
        let l = json!([{"loader":{"version":"0.16.0-beta.1","stable":false}},{"loader":{"version":"0.15.11","stable":true}}]);
        assert_eq!(pick_loader_version(&l).unwrap(), "0.15.11");
        let only_beta = json!([{"loader":{"version":"0.1-beta","stable":false}}]);
        assert_eq!(pick_loader_version(&only_beta).unwrap(), "0.1-beta");
        assert!(pick_loader_version(&json!([])).is_none());
    }

    #[test]
    fn forge_urls() {
        let p = json!({"promos":{"1.20.1-latest":"47.3.0","1.20.1-recommended":"47.2.0","1.7.10-latest":"10.13.4.1614"}});
        assert_eq!(forge_version(&p, "1.20.1").unwrap(), "47.2.0");
        assert_eq!(forge_version(&p, "1.7.10").unwrap(), "10.13.4.1614");
        assert!(forge_version(&p, "9.9").is_none());
        assert_eq!(forge_installer_url("1.20.1", "47.2.0"), "https://maven.minecraftforge.net/net/minecraftforge/forge/1.20.1-47.2.0/forge-1.20.1-47.2.0-installer.jar");
    }

    #[test]
    fn neoforge_mapping() {
        assert_eq!(neoforge_prefix("1.21.1").unwrap(), "21.1.");
        assert_eq!(neoforge_prefix("1.21").unwrap(), "21.0.");
        assert_eq!(neoforge_prefix("1.20.4").unwrap(), "20.4.");
        assert!(neoforge_prefix("26.1").is_none());
        let v: Vec<String> = ["21.1.9", "21.1.100", "21.1.200-beta", "21.0.5", "20.4.1"].iter().map(|s| s.to_string()).collect();
        assert_eq!(neoforge_version(&v, "1.21.1").unwrap(), "21.1.100", "числовая сортировка, beta ниже стабильных");
        assert_eq!(neoforge_version(&v, "1.21").unwrap(), "21.0.5");
        assert!(neoforge_version(&v, "1.19").is_none());
    }

    #[test]
    fn profile_id_traversal_rejected() {
        let d = Dirs { root: std::env::temp_dir().join(format!("rs-ld-{}", std::process::id())) };
        assert!(write_profile(&d, "../x", &json!({})).is_err());
        write_profile(&d, "fabric-loader-0.15-1.20.1", &json!({"id":"x"})).unwrap();
        assert!(list_versions(&d).contains("fabric-loader-0.15-1.20.1"));
    }

    #[cfg(unix)]
    #[test]
    fn installer_detects_new_version_and_failure() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("rs-inst2-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let d = Dirs { root: root.clone() };
        // «java»-заглушка: создаёт versions/1.20.1-forge-47.2.0 при --installClient
        let fake = root.join("fakejava.sh");
        fs::write(&fake, format!("#!/bin/sh\nmkdir -p {}/versions/1.20.1-forge-47.2.0\necho installed\n", root.display())).unwrap();
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
        let id = run_installer(&fake, Path::new("x.jar"), &d, &|_| {}).unwrap();
        assert_eq!(id, "1.20.1-forge-47.2.0");
        assert!(root.join("launcher_profiles.json").is_file());
        let bad = root.join("bad.sh");
        fs::write(&bad, "#!/bin/sh\necho boom\nexit 1\n").unwrap();
        fs::set_permissions(&bad, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(run_installer(&bad, Path::new("x.jar"), &d, &|_| {}).unwrap_err().contains("boom"));
    }
}

//! Установка версии: manifest, client.jar, библиотеки, ассеты, natives.

use crate::launch::{self, Env, Lib};
use crate::net::{self, Job, Result};
use serde_json::Value;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

pub const MANIFEST_URL: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
pub const RES_URL: &str = "https://resources.download.minecraft.net";

#[derive(Clone, Debug)]
pub struct Dirs { pub root: PathBuf }

impl Dirs {
    pub fn libraries(&self) -> PathBuf { self.root.join("libraries") }
    pub fn assets(&self) -> PathBuf { self.root.join("assets") }
    pub fn versions(&self) -> PathBuf { self.root.join("versions") }
    pub fn runtimes(&self) -> PathBuf { self.root.join("runtime") }
    pub fn version_dir(&self, id: &str) -> PathBuf { self.versions().join(id) }
    pub fn version_json(&self, id: &str) -> PathBuf { self.version_dir(id).join(format!("{id}.json")) }
    pub fn client_jar(&self, id: &str) -> PathBuf { self.version_dir(id).join(format!("{id}.jar")) }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionEntry { pub id: String, pub kind: String, pub url: String }

pub fn parse_manifest(v: &Value) -> Vec<VersionEntry> {
    v.get("versions").and_then(|a| a.as_array()).into_iter().flatten().filter_map(|e| Some(VersionEntry {
        id: e.get("id")?.as_str()?.to_string(),
        kind: e.get("type")?.as_str()?.to_string(),
        url: e.get("url")?.as_str()?.to_string(),
    })).collect()
}

/// Читает локальный JSON версии, при наличии `inheritsFrom` — сливает с родителем (рекурсивно).
pub fn load_resolved(dirs: &Dirs, id: &str) -> Result<Value> {
    let read = |id: &str| -> Result<Value> {
        let p = dirs.version_json(id);
        let s = fs::read_to_string(&p).map_err(|e| format!("нет {}: {e}", p.display()))?;
        serde_json::from_str(&s).map_err(|e| format!("битый JSON {}: {e}", p.display()))
    };
    let mut v = read(id)?;
    let mut base: Option<String> = None;
    let mut depth = 0;
    while let Some(parent) = v.get("inheritsFrom").and_then(|p| p.as_str()).map(String::from) {
        depth += 1;
        if depth > 5 { return Err("слишком глубокая цепочка inheritsFrom".into()); }
        let p = read(&parent)?;
        base = Some(parent.clone());
        let mut merged = launch::merge(&p, &v);
        // продолжаем вверх, если у родителя тоже есть inheritsFrom
        if let Some(pp) = p.get("inheritsFrom") { merged["inheritsFrom"] = pp.clone(); }
        v = merged;
    }
    if let Some(b) = base { v["_redshiftBase"] = Value::String(b); }
    Ok(v)
}

/// Базовая (ванильная) версия, из которой берём client.jar: последний `jar` либо id родителя.
pub fn jar_id(v: &Value, id: &str) -> String {
    // client.jar берём у ванильной базовой версии (Fabric/Forge-профили своего jar не имеют).
    v.get("jar").and_then(|j| j.as_str()).map(String::from)
        .or_else(|| v.get("_redshiftBase").and_then(|j| j.as_str()).map(String::from))
        .unwrap_or_else(|| id.to_string())
}

pub fn library_jobs(dirs: &Dirs, libs: &[Lib]) -> (Vec<Job>, Vec<String>) {
    let mut jobs = vec![]; let mut missing = vec![];
    for l in libs {
        let dest = dirs.libraries().join(&l.path);
        match &l.url {
            Some(u) => jobs.push(Job { url: u.clone(), dest, sha1: l.sha1.clone() }),
            None => if !dest.is_file() { missing.push(l.name.clone()) },
        }
    }
    (jobs, missing)
}

pub fn asset_jobs(dirs: &Dirs, index: &Value) -> Vec<Job> {
    let mut seen = std::collections::HashSet::new();
    let mut jobs = vec![];
    for (_, o) in index.get("objects").and_then(|o| o.as_object()).into_iter().flatten() {
        let Some(h) = o.get("hash").and_then(|h| h.as_str()) else { continue };
        if h.len() < 2 || !seen.insert(h.to_string()) { continue; }
        jobs.push(Job { url: format!("{RES_URL}/{}/{h}", &h[..2]), dest: dirs.assets().join("objects").join(&h[..2]).join(h), sha1: Some(h.to_string()) });
    }
    jobs
}

/// Для legacy/pre-1.6 индексов копирует объекты в assets/virtual/<id>/<name> (или game_dir/resources).
pub fn materialize_virtual(dirs: &Dirs, index: &Value, index_id: &str, game_dir: &Path) -> Result<()> {
    let virt = index.get("virtual").and_then(|v| v.as_bool()).unwrap_or(false);
    let map = index.get("map_to_resources").and_then(|v| v.as_bool()).unwrap_or(false);
    if !virt && !map { return Ok(()); }
    let target = if map { game_dir.join("resources") } else { dirs.assets().join("virtual").join(index_id) };
    for (name, o) in index.get("objects").and_then(|o| o.as_object()).into_iter().flatten() {
        let Some(h) = o.get("hash").and_then(|h| h.as_str()) else { continue };
        if h.len() < 2 { continue; }
        let rel = safe_rel(name).ok_or_else(|| format!("небезопасный путь ассета: {name}"))?;
        let dest = target.join(rel);
        if dest.is_file() { continue; }
        let src = dirs.assets().join("objects").join(&h[..2]).join(h);
        if let Some(p) = dest.parent() { fs::create_dir_all(p).map_err(|e| e.to_string())?; }
        fs::copy(&src, &dest).map_err(|e| format!("{}: {e}", src.display()))?;
    }
    Ok(())
}

/// Относительный путь без `..`/абсолютных частей.
pub fn safe_rel(name: &str) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for part in name.split(['/', '\\']) {
        match part { "" | "." => {}, ".." => return None, p if p.contains(':') => return None, p => out.push(p) }
    }
    if out.as_os_str().is_empty() { None } else { Some(out) }
}

/// Распаковка natives (старый формат). Пропускает META-INF и `exclude`, защищена от zip-slip.
pub fn extract_natives(dirs: &Dirs, libs: &[Lib], dest: &Path) -> Result<usize> {
    fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    let mut n = 0;
    for l in libs.iter().filter(|l| l.native) {
        let jar = dirs.libraries().join(&l.path);
        let f = fs::File::open(&jar).map_err(|e| format!("{}: {e}", jar.display()))?;
        let mut z = zip::ZipArchive::new(f).map_err(|e| format!("{}: {e}", jar.display()))?;
        for i in 0..z.len() {
            let mut e = z.by_index(i).map_err(|e| e.to_string())?;
            if e.is_dir() { continue; }
            let name = e.name().to_string();
            if name.starts_with("META-INF/") || l.extract_exclude.iter().any(|x| name.starts_with(x.as_str())) { continue; }
            let Some(rel) = safe_rel(&name) else { continue };
            let out = dest.join(rel);
            if let Some(p) = out.parent() { fs::create_dir_all(p).map_err(|e| e.to_string())?; }
            let mut buf = vec![]; e.read_to_end(&mut buf).map_err(|e| e.to_string())?;
            fs::write(&out, buf).map_err(|e| e.to_string())?;
            n += 1;
        }
    }
    Ok(n)
}

/// Полная установка ванильной/профильной версии, у которой уже лежит JSON.
pub fn ensure_version_files(agent: &ureq::Agent, dirs: &Dirs, id: &str, env: &Env, threads: usize,
                            progress: &(dyn Fn(&str, usize, usize) + Sync)) -> Result<Value> {
    let v = load_resolved(dirs, id)?;
    // client.jar
    let jid = jar_id(&v, id);
    let jar = dirs.client_jar(&jid);
    let own_jar = dirs.client_jar(id);
    let target = if jar.is_file() { jar } else { own_jar };
    if !target.is_file() {
        let d = v.pointer("/downloads/client").ok_or("в JSON нет downloads.client")?;
        let url = d.get("url").and_then(|u| u.as_str()).ok_or("нет url клиента")?;
        let job = Job { url: url.into(), dest: target.clone(), sha1: d.get("sha1").and_then(|s| s.as_str()).map(String::from) };
        net::download_one(agent, &job)?;
    }
    // libraries
    let libs = launch::libraries(&v, env);
    let (jobs, missing) = library_jobs(dirs, &libs);
    if !missing.is_empty() { return Err(format!("нет источника для библиотек: {}", missing.join(", "))); }
    net::download_all(agent, jobs, threads, &|d, t| progress("Библиотеки", d, t))?;
    // assets
    let idx = v.get("assetIndex").ok_or("нет assetIndex")?;
    let idx_id = idx.get("id").and_then(|i| i.as_str()).unwrap_or("legacy");
    let idx_path = dirs.assets().join("indexes").join(format!("{idx_id}.json"));
    if let Some(u) = idx.get("url").and_then(|u| u.as_str()) {
        net::download_one(agent, &Job { url: u.into(), dest: idx_path.clone(), sha1: idx.get("sha1").and_then(|s| s.as_str()).map(String::from) })?;
    }
    let index: Value = serde_json::from_str(&fs::read_to_string(&idx_path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    net::download_all(agent, asset_jobs(dirs, &index), threads, &|d, t| progress("Ресурсы", d, t))?;
    Ok(v)
}

/// Путь к client.jar, который реально существует для данной (возможно, профильной) версии.
pub fn resolved_client_jar(dirs: &Dirs, v: &Value, id: &str) -> PathBuf {
    let j = dirs.client_jar(&jar_id(v, id));
    if j.is_file() { j } else { dirs.client_jar(id) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;

    fn tmp(n: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rs-inst-{n}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d); fs::create_dir_all(&d).unwrap(); d
    }

    #[test]
    fn manifest_parse() {
        let v = json!({"versions":[{"id":"1.20.1","type":"release","url":"u"},{"id":"b1.7.3","type":"old_beta","url":"u2"},{"broken":1}]});
        let m = parse_manifest(&v);
        assert_eq!(m.len(), 2);
        assert_eq!(m[1].kind, "old_beta");
    }

    #[test]
    fn inherits_chain_resolved() {
        let d = Dirs { root: tmp("inh") };
        for (id, j) in [("1.20.1", json!({"id":"1.20.1","mainClass":"A","libraries":[{"name":"a:b:1"}],"arguments":{"game":["x"],"jvm":[]}})),
                        ("fabric-1.20.1", json!({"id":"fabric-1.20.1","inheritsFrom":"1.20.1","mainClass":"K","libraries":[{"name":"c:d:2"}],"arguments":{"game":["y"]}}))] {
            fs::create_dir_all(d.version_dir(id)).unwrap();
            fs::write(d.version_json(id), j.to_string()).unwrap();
        }
        let v = load_resolved(&d, "fabric-1.20.1").unwrap();
        assert_eq!(v["mainClass"], "K");
        assert_eq!(v["libraries"].as_array().unwrap().len(), 2);
        assert_eq!(v["arguments"]["game"], json!(["x", "y"]));
        assert_eq!(jar_id(&v, "fabric-1.20.1"), "1.20.1");
    }

    #[test]
    fn inherits_cycle_is_rejected() {
        let d = Dirs { root: tmp("cyc") };
        for id in ["a", "b"] {
            fs::create_dir_all(d.version_dir(id)).unwrap();
            fs::write(d.version_json(id), json!({"inheritsFrom": if id == "a" {"b"} else {"a"}, "mainClass":"M"}).to_string()).unwrap();
        }
        assert!(load_resolved(&d, "a").is_err());
    }

    #[test]
    fn safe_rel_blocks_traversal() {
        assert!(safe_rel("../evil").is_none());
        assert!(safe_rel("a/../../b").is_none());
        assert!(safe_rel("C:\\x").is_none());
        assert_eq!(safe_rel("icons/a.png").unwrap(), PathBuf::from("icons/a.png"));
    }

    #[test]
    fn natives_extract_and_slip_guard() {
        let d = Dirs { root: tmp("nat") };
        let jar = d.libraries().join("o/n.jar");
        fs::create_dir_all(jar.parent().unwrap()).unwrap();
        let mut z = zip::ZipWriter::new(fs::File::create(&jar).unwrap());
        let o = zip::write::SimpleFileOptions::default();
        for (n, c) in [("lwjgl.dll", "A"), ("META-INF/x.sf", "B"), ("skip/me.txt", "C"), ("../escape.dll", "D")] {
            z.start_file(n, o).unwrap(); z.write_all(c.as_bytes()).unwrap();
        }
        z.finish().unwrap();
        let lib = Lib { name: "n".into(), path: "o/n.jar".into(), url: None, sha1: None, native: true, extract_exclude: vec!["skip/".into()] };
        let dest = d.root.join("natives");
        let n = extract_natives(&d, &[lib], &dest).unwrap();
        assert_eq!(n, 1);
        assert!(dest.join("lwjgl.dll").is_file());
        assert!(!dest.join("META-INF").exists() && !dest.join("skip").exists());
        assert!(!d.root.join("escape.dll").exists());
    }

    #[test]
    fn virtual_assets_materialized() {
        let d = Dirs { root: tmp("virt") };
        let h = "ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12";
        let obj = d.assets().join("objects/ab").join(h);
        fs::create_dir_all(obj.parent().unwrap()).unwrap();
        fs::write(&obj, "snd").unwrap();
        let idx = json!({"virtual":true,"objects":{"sound/a.ogg":{"hash":h},"../bad":{"hash":h}}});
        assert!(materialize_virtual(&d, &idx, "legacy", &d.root).is_err(), "traversal в имени ассета отклоняется");
        let idx = json!({"virtual":true,"objects":{"sound/a.ogg":{"hash":h}}});
        materialize_virtual(&d, &idx, "legacy", &d.root).unwrap();
        assert_eq!(fs::read_to_string(d.assets().join("virtual/legacy/sound/a.ogg")).unwrap(), "snd");
    }

    #[test]
    fn asset_jobs_dedup() {
        let d = Dirs { root: tmp("aj") };
        let h = "cd".repeat(20);
        let jobs = asset_jobs(&d, &json!({"objects":{"a":{"hash":h},"b":{"hash":h}}}));
        assert_eq!(jobs.len(), 1);
        assert!(jobs[0].url.starts_with(RES_URL));
    }
}

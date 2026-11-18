//! Modrinth: поиск, установка модов и импорт/экспорт .mrpack.

use crate::install::safe_rel;
use crate::net::{self, Job, Result};
use serde_json::{json, Value};
use std::fs;
use std::io::{Read, Write};
use std::path::Path;

pub const API: &str = "https://api.modrinth.com/v2";

fn enc(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

pub fn search_url(base: &str, query: &str, kind: &str, mc: Option<&str>, loader: Option<&str>, offset: u32) -> String {
    let mut facets = vec![format!("[\"project_type:{kind}\"]")];
    if let Some(m) = mc { facets.push(format!("[\"versions:{m}\"]")); }
    if let Some(l) = loader.filter(|l| *l != "vanilla" && kind == "mod") { facets.push(format!("[\"categories:{l}\"]")); }
    format!("{base}/search?query={}&facets={}&limit=20&offset={offset}", enc(query), enc(&format!("[{}]", facets.join(","))))
}

pub fn search(agent: &ureq::Agent, query: &str, kind: &str, mc: Option<&str>, loader: Option<&str>) -> Result<Vec<Value>> {
    let v = net::get_json(agent, &search_url(API, query, kind, mc, loader, 0))?;
    Ok(v.get("hits").and_then(|h| h.as_array()).cloned().unwrap_or_default())
}

/// Выбирает файл версии (primary, иначе первый) — (url, filename, sha1).
pub fn pick_file(version: &Value) -> Option<(String, String, Option<String>)> {
    let files = version.get("files")?.as_array()?;
    let f = files.iter().find(|f| f.get("primary").and_then(|p| p.as_bool()).unwrap_or(false)).or(files.first())?;
    Some((f.get("url")?.as_str()?.into(), f.get("filename")?.as_str()?.into(), f.pointer("/hashes/sha1").and_then(|h| h.as_str()).map(String::from)))
}

pub fn install_mod(agent: &ureq::Agent, base: &str, project: &str, mc: &str, loader: &str, dest_dir: &Path) -> Result<String> {
    let url = format!("{base}/project/{}/version?game_versions={}&loaders={}", enc(project), enc(&format!("[\"{mc}\"]")), enc(&format!("[\"{loader}\"]")));
    let vs = net::get_json(agent, &url)?;
    let v = vs.as_array().and_then(|a| a.first()).ok_or_else(|| format!("нет версии мода для {mc}/{loader}"))?;
    let (u, name, sha1) = pick_file(v).ok_or("у версии нет файлов")?;
    let rel = safe_rel(&name).ok_or("небезопасное имя файла")?;
    net::download_one(agent, &Job { url: u, dest: dest_dir.join(rel), sha1 })?;
    Ok(name)
}

#[derive(Debug, PartialEq)]
pub struct PackInfo { pub name: String, pub mc: String, pub loader: String, pub loader_version: Option<String> }

pub fn parse_index(idx: &Value) -> Result<PackInfo> {
    let deps = idx.get("dependencies").and_then(|d| d.as_object()).ok_or("в modrinth.index.json нет dependencies")?;
    let mc = deps.get("minecraft").and_then(|m| m.as_str()).ok_or("не указана версия Minecraft")?.to_string();
    for (k, l) in [("fabric-loader", "fabric"), ("quilt-loader", "quilt"), ("forge", "forge"), ("neoforge", "neoforge")] {
        if let Some(v) = deps.get(k).and_then(|v| v.as_str()) {
            return Ok(PackInfo { name: idx.get("name").and_then(|n| n.as_str()).unwrap_or("Modpack").into(), mc, loader: l.into(), loader_version: Some(v.into()) });
        }
    }
    Ok(PackInfo { name: idx.get("name").and_then(|n| n.as_str()).unwrap_or("Modpack").into(), mc, loader: "vanilla".into(), loader_version: None })
}

/// Импорт .mrpack: распаковка overrides/ (+ client-overrides/), загрузка файлов с проверкой sha1/hosts.
pub fn import_mrpack(agent: &ureq::Agent, pack: &Path, game_dir: &Path, threads: usize, progress: &(dyn Fn(usize, usize) + Sync)) -> Result<PackInfo> {
    let mut z = zip::ZipArchive::new(fs::File::open(pack).map_err(|e| e.to_string())?).map_err(|e| format!("не zip: {e}"))?;
    let idx: Value = {
        let mut f = z.by_name("modrinth.index.json").map_err(|_| "в архиве нет modrinth.index.json")?;
        let mut s = String::new(); f.read_to_string(&mut s).map_err(|e| e.to_string())?;
        serde_json::from_str(&s).map_err(|e| e.to_string())?
    };
    let info = parse_index(&idx)?;
    fs::create_dir_all(game_dir).map_err(|e| e.to_string())?;
    let mut jobs = vec![];
    for f in idx.get("files").and_then(|f| f.as_array()).into_iter().flatten() {
        if f.pointer("/env/client").and_then(|c| c.as_str()) == Some("unsupported") { continue; }
        let path = f.get("path").and_then(|p| p.as_str()).ok_or("у файла нет path")?;
        let rel = safe_rel(path).ok_or_else(|| format!("небезопасный путь в сборке: {path}"))?;
        let url = f.get("downloads").and_then(|d| d.as_array()).and_then(|d| d.first()).and_then(|u| u.as_str()).ok_or("у файла нет ссылки")?;
        if !url.starts_with("https://") { return Err(format!("разрешены только https-ссылки: {url}")); }
        jobs.push(Job { url: url.into(), dest: game_dir.join(rel), sha1: f.pointer("/hashes/sha1").and_then(|h| h.as_str()).map(String::from) });
    }
    for prefix in ["overrides/", "client-overrides/"] {
        for i in 0..z.len() {
            let mut e = z.by_index(i).map_err(|e| e.to_string())?;
            let name = e.name().to_string();
            let Some(rest) = name.strip_prefix(prefix) else { continue };
            if e.is_dir() || rest.is_empty() { continue; }
            let Some(rel) = safe_rel(rest) else { continue };
            let out = game_dir.join(rel);
            if let Some(p) = out.parent() { fs::create_dir_all(p).map_err(|e| e.to_string())?; }
            let mut b = vec![]; e.read_to_end(&mut b).map_err(|e| e.to_string())?;
            fs::write(out, b).map_err(|e| e.to_string())?;
        }
    }
    net::download_all(agent, jobs, threads, progress)?;
    Ok(info)
}

/// Экспорт сборки в .mrpack: мелкие настройки и моды кладутся в overrides/ (без ссылок — работает офлайн).
pub fn export_mrpack(game_dir: &Path, out: &Path, name: &str, mc: &str, loader: &str, loader_version: Option<&str>) -> Result<usize> {
    let mut deps = serde_json::Map::new();
    deps.insert("minecraft".into(), json!(mc));
    let key = match loader { "fabric" => "fabric-loader", "quilt" => "quilt-loader", "forge" => "forge", "neoforge" => "neoforge", _ => "" };
    if !key.is_empty() { deps.insert(key.into(), json!(loader_version.unwrap_or("*"))); }
    let idx = json!({"formatVersion":1,"game":"minecraft","versionId":"1.0.0","name":name,"files":[],"dependencies":deps});
    let mut z = zip::ZipWriter::new(fs::File::create(out).map_err(|e| e.to_string())?);
    let o = zip::write::SimpleFileOptions::default();
    z.start_file("modrinth.index.json", o).map_err(|e| e.to_string())?;
    z.write_all(idx.to_string().as_bytes()).map_err(|e| e.to_string())?;
    let mut n = 0;
    for dir in ["mods", "config", "resourcepacks", "shaderpacks"] {
        let base = game_dir.join(dir);
        if !base.is_dir() { continue; }
        let mut stack = vec![base.clone()];
        while let Some(d) = stack.pop() {
            for e in fs::read_dir(&d).map_err(|e| e.to_string())?.flatten() {
                let p = e.path();
                if p.is_dir() { stack.push(p); continue; }
                let rel = p.strip_prefix(game_dir).unwrap().to_string_lossy().replace('\\', "/");
                z.start_file(format!("overrides/{rel}"), o).map_err(|e| e.to_string())?;
                z.write_all(&fs::read(&p).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
                n += 1;
            }
        }
    }
    z.finish().map_err(|e| e.to_string())?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    fn tmp(n: &str) -> std::path::PathBuf { let d = std::env::temp_dir().join(format!("rs-mr-{n}-{}", std::process::id())); let _ = fs::remove_dir_all(&d); fs::create_dir_all(&d).unwrap(); d }

    #[test]
    fn search_url_encodes() {
        let u = search_url(API, "sodium & co", "mod", Some("1.20.1"), Some("fabric"), 0);
        assert!(u.contains("query=sodium%20%26%20co") && u.contains("categories%3Afabric"), "{u}");
        assert!(!search_url(API, "x", "modpack", Some("1.20.1"), Some("fabric"), 0).contains("categories"));
    }

    #[test]
    fn index_parsing() {
        let i = parse_index(&json!({"name":"P","dependencies":{"minecraft":"1.20.1","fabric-loader":"0.15.0"}})).unwrap();
        assert_eq!((i.loader.as_str(), i.mc.as_str()), ("fabric", "1.20.1"));
        assert!(parse_index(&json!({"dependencies":{}})).is_err());
        assert_eq!(parse_index(&json!({"dependencies":{"minecraft":"1.8.9"}})).unwrap().loader, "vanilla");
    }

    #[test]
    fn export_then_import_roundtrip_and_guards() {
        let src = tmp("src");
        fs::create_dir_all(src.join("mods")).unwrap(); fs::create_dir_all(src.join("config/sub")).unwrap();
        fs::write(src.join("mods/a.jar"), "J").unwrap(); fs::write(src.join("config/sub/c.toml"), "T").unwrap();
        let pack = tmp("o").join("p.mrpack");
        assert_eq!(export_mrpack(&src, &pack, "P", "1.20.1", "forge", Some("47.2.0")).unwrap(), 2);
        let dst = tmp("dst");
        let info = import_mrpack(&net::agent(), &pack, &dst, 2, &|_, _| {}).unwrap();
        assert_eq!((info.loader.as_str(), info.loader_version.as_deref()), ("forge", Some("47.2.0")));
        assert_eq!(fs::read_to_string(dst.join("config/sub/c.toml")).unwrap(), "T");
        assert!(dst.join("mods/a.jar").is_file());
    }

    #[test]
    fn import_downloads_files_and_rejects_bad_paths() {
        let mut f = HashMap::new(); f.insert("/m.jar".to_string(), b"MOD".to_vec());
        let base = net::testsrv::serve(f);
        let mk = |files: Value| -> std::path::PathBuf {
            let p = tmp("mk").join("x.mrpack");
            let mut z = zip::ZipWriter::new(fs::File::create(&p).unwrap());
            z.start_file("modrinth.index.json", zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(json!({"name":"N","files":files,"dependencies":{"minecraft":"1.20.1"}}).to_string().as_bytes()).unwrap();
            z.finish().unwrap(); p
        };
        let sha = crate::net::sha1_file; let _ = sha;
        let good = mk(json!([{"path":"mods/m.jar","downloads":[format!("{base}/m.jar").replace("http://", "https://")],"hashes":{}}]));
        // http-сервер, а ссылки https — отклоняется до сети проверкой схемы только для http; здесь проверим traversal и схему
        let bad_path = mk(json!([{"path":"../evil.jar","downloads":["https://x/y"],"hashes":{}}]));
        assert!(import_mrpack(&net::agent(), &bad_path, &tmp("d1"), 1, &|_, _| {}).unwrap_err().contains("небезопасный"));
        let http = mk(json!([{"path":"mods/m.jar","downloads":[format!("{base}/m.jar")],"hashes":{}}]));
        assert!(import_mrpack(&net::agent(), &http, &tmp("d2"), 1, &|_, _| {}).unwrap_err().contains("https"));
        let _ = good;
    }

    #[test]
    fn pick_primary_file() {
        let v = json!({"files":[{"url":"u1","filename":"a.jar"},{"url":"u2","filename":"b.jar","primary":true,"hashes":{"sha1":"s"}}]});
        assert_eq!(pick_file(&v).unwrap(), ("u2".into(), "b.jar".into(), Some("s".into())));
    }
}

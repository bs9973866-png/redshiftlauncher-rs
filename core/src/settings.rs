//! Настройки, сборки (instances) и аккаунты — один JSON-файл, запись атомарная.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Settings {
    /// 0 = авто по объёму RAM ПК
    pub ram_mb: u32,
    /// auto | safe | balanced | max | lowend
    pub jvm_preset: String,
    /// Облегчённый режим интерфейса (по умолчанию включён: тихий, без анимаций)
    pub lite: bool,
    /// Потоков скачивания
    pub download_threads: u32,
    pub close_launcher_on_play: bool,
    pub selected_instance: Option<String>,
    pub selected_account: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { ram_mb: 0, jvm_preset: "auto".into(), lite: true, download_threads: 4, close_launcher_on_play: false,
                   selected_instance: None, selected_account: None }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct Instance {
    pub id: String,
    pub name: String,
    pub mc: String,
    /// vanilla | fabric | quilt | forge | neoforge
    pub loader: String,
    pub loader_version: Option<String>,
    /// Установленный id версии для запуска (после установки загрузчика)
    pub launch_id: Option<String>,
    /// 0 = как в настройках
    pub ram_mb: u32,
    /// Много модов — выше RAM по умолчанию
    pub heavy: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct Account {
    pub id: String,
    /// offline | ely
    pub kind: String,
    pub name: String,
    pub uuid: String,
    pub access_token: Option<String>,
    pub client_token: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Store { pub settings: Settings, pub instances: Vec<Instance>, pub accounts: Vec<Account> }

pub fn path(root: &Path) -> PathBuf { root.join("redshift.json") }

pub fn load(root: &Path) -> Store {
    // Повреждённый файл не должен ломать запуск: пробуем .bak, иначе значения по умолчанию.
    for p in [path(root), path(root).with_extension("json.bak")] {
        if let Ok(s) = fs::read_to_string(&p) {
            if let Ok(st) = serde_json::from_str::<Store>(&s) { return st; }
        }
    }
    Store::default()
}

pub fn save(root: &Path, st: &Store) -> Result<(), String> {
    fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let p = path(root);
    let tmp = p.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(st).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    if p.exists() { let _ = fs::copy(&p, p.with_extension("json.bak")); }
    fs::rename(&tmp, &p).map_err(|e| e.to_string())
}

pub fn slug(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    let s = s.split('-').filter(|x| !x.is_empty()).collect::<Vec<_>>().join("-");
    if s.is_empty() { "instance".into() } else { s }
}

pub fn unique_id(existing: &[Instance], name: &str) -> String {
    let base = slug(name);
    let mut id = base.clone(); let mut n = 2;
    while existing.iter().any(|i| i.id == id) { id = format!("{base}-{n}"); n += 1; }
    id
}

pub fn instance_dir(root: &Path, id: &str) -> PathBuf { root.join("instances").join(id) }

#[cfg(test)]
mod tests {
    use super::*;
    fn tmp(n: &str) -> PathBuf { let d = std::env::temp_dir().join(format!("rs-set-{n}-{}", std::process::id())); let _ = fs::remove_dir_all(&d); d }

    #[test]
    fn roundtrip_and_defaults() {
        let d = tmp("rt");
        assert_eq!(load(&d), Store::default());
        let mut st = Store::default();
        st.instances.push(Instance { id: "a".into(), name: "A".into(), mc: "1.20.1".into(), loader: "forge".into(), ..Default::default() });
        save(&d, &st).unwrap();
        assert_eq!(load(&d), st);
        assert!(Settings::default().lite, "лёгкий режим включён по умолчанию");
    }

    #[test]
    fn partial_json_gets_defaults() {
        let d = tmp("part");
        fs::create_dir_all(&d).unwrap();
        fs::write(path(&d), r#"{"settings":{"ram_mb":2048}}"#).unwrap();
        let st = load(&d);
        assert_eq!(st.settings.ram_mb, 2048);
        assert_eq!(st.settings.download_threads, 4);
    }

    #[test]
    fn corrupt_falls_back_to_backup() {
        let d = tmp("cor");
        let mut st = Store::default(); st.settings.ram_mb = 3000;
        save(&d, &st).unwrap();
        save(&d, &st).unwrap(); // создаёт .bak
        fs::write(path(&d), "{ broken").unwrap();
        assert_eq!(load(&d).settings.ram_mb, 3000);
        fs::write(path(&d).with_extension("json.bak"), "also broken").unwrap();
        assert_eq!(load(&d), Store::default());
    }

    #[test]
    fn ids_unique_and_safe() {
        assert_eq!(slug("My Pack: 1.20!"), "my-pack-1-20");
        assert_eq!(slug("../../x"), "x");
        assert_eq!(slug("!!!"), "instance");
        let ex = vec![Instance { id: "pack".into(), ..Default::default() }];
        assert_eq!(unique_id(&ex, "Pack"), "pack-2");
    }
}

//! Сборка команды запуска Minecraft из JSON версии Mojang (+ Fabric/Quilt/Forge профили).
//! Заменяет minecraft-launcher-core. Без сети и ввода-вывода — чистая логика, поэтому тестируется.

use serde_json::{json, Value};
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os { Windows, Linux, Osx }

impl Os {
    pub fn current() -> Os {
        if cfg!(windows) { Os::Windows } else if cfg!(target_os = "macos") { Os::Osx } else { Os::Linux }
    }
    pub fn name(self) -> &'static str { match self { Os::Windows => "windows", Os::Linux => "linux", Os::Osx => "osx" } }
    pub fn cp_sep(self) -> &'static str { if self == Os::Windows { ";" } else { ":" } }
}

#[derive(Clone, Debug)]
pub struct Env { pub os: Os, pub arch64: bool, pub demo: bool, pub custom_resolution: bool }

impl Env {
    pub fn new(os: Os) -> Env { Env { os, arch64: true, demo: false, custom_resolution: false } }
}

/// Проверка массива `rules` (allow/deny + os/features).
pub fn rules_allow(rules: Option<&Value>, env: &Env) -> bool {
    let Some(arr) = rules.and_then(|r| r.as_array()) else { return true };
    if arr.is_empty() { return true; }
    let mut allowed = false;
    for r in arr {
        let action_allow = r.get("action").and_then(|a| a.as_str()) != Some("deny");
        if rule_matches(r, env) { allowed = action_allow; }
    }
    allowed
}

fn rule_matches(r: &Value, env: &Env) -> bool {
    if let Some(os) = r.get("os") {
        if let Some(n) = os.get("name").and_then(|n| n.as_str()) {
            if n != env.os.name() { return false; }
        }
        if let Some(a) = os.get("arch").and_then(|n| n.as_str()) {
            let is32 = a == "x86";
            if is32 == env.arch64 { return false; }
        }
    }
    if let Some(f) = r.get("features").and_then(|f| f.as_object()) {
        for (k, v) in f {
            let want = v.as_bool().unwrap_or(false);
            let have = match k.as_str() {
                "is_demo_user" => env.demo,
                "has_custom_resolution" => env.custom_resolution,
                _ => false,
            };
            if want != have { return false; }
        }
    }
    true
}

/// Слияние профиля с `inheritsFrom` (child поверх parent).
pub fn merge(parent: &Value, child: &Value) -> Value {
    let mut out = parent.clone();
    let o = out.as_object_mut().unwrap();
    for (k, v) in child.as_object().into_iter().flatten() {
        match k.as_str() {
            "libraries" => {
                let mut libs: Vec<Value> = v.as_array().cloned().unwrap_or_default();
                libs.extend(parent.get("libraries").and_then(|l| l.as_array()).cloned().unwrap_or_default());
                o.insert(k.clone(), Value::Array(libs));
            }
            "arguments" => {
                let mut args = parent.get("arguments").cloned().unwrap_or_else(|| json!({}));
                for part in ["game", "jvm"] {
                    let mut a = args.get(part).and_then(|x| x.as_array()).cloned().unwrap_or_default();
                    a.extend(v.get(part).and_then(|x| x.as_array()).cloned().unwrap_or_default());
                    args[part] = Value::Array(a);
                }
                o.insert(k.clone(), args);
            }
            "minecraftArguments" => {
                // Старый формат: дочерний заменяет родительский целиком (так делает Forge для 1.7-1.12).
                o.insert(k.clone(), v.clone());
            }
            "inheritsFrom" => {}
            _ => { o.insert(k.clone(), v.clone()); }
        }
    }
    out
}

/// "g:a:v[:classifier][@ext]" -> относительный путь в libraries/.
pub fn maven_path(name: &str) -> Option<String> {
    let (coords, ext) = match name.split_once('@') { Some((c, e)) => (c, e), None => (name, "jar") };
    let p: Vec<&str> = coords.split(':').collect();
    if p.len() < 3 { return None; }
    let (g, a, v) = (p[0].replace('.', "/"), p[1], p[2]);
    let cls = p.get(3).map(|c| format!("-{c}")).unwrap_or_default();
    Some(format!("{g}/{a}/{v}/{a}-{v}{cls}.{ext}"))
}

/// Ключ дедупликации: group:artifact[:classifier] без версии.
pub fn lib_key(name: &str) -> String {
    let coords = name.split('@').next().unwrap_or(name);
    let p: Vec<&str> = coords.split(':').collect();
    match p.len() {
        0..=2 => coords.to_string(),
        3 => format!("{}:{}", p[0], p[1]),
        _ => format!("{}:{}:{}", p[0], p[1], p[3]),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lib {
    pub name: String,
    pub path: String,           // относительно libraries/
    pub url: Option<String>,
    pub sha1: Option<String>,
    pub native: bool,           // нужно распаковать в natives_dir (старый формат)
    pub extract_exclude: Vec<String>,
}

/// Список библиотек для данной ОС: фильтр правил, дедуп (первый побеждает — а child идёт первым).
pub fn libraries(v: &Value, env: &Env) -> Vec<Lib> {
    let mut out = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for l in v.get("libraries").and_then(|x| x.as_array()).into_iter().flatten() {
        if !rules_allow(l.get("rules"), env) { continue; }
        let Some(name) = l.get("name").and_then(|n| n.as_str()) else { continue };
        let dl = l.get("downloads");
        let excl: Vec<String> = l.pointer("/extract/exclude").and_then(|e| e.as_array())
            .map(|a| a.iter().filter_map(|s| s.as_str().map(String::from)).collect()).unwrap_or_default();

        // Старый формат natives
        if let Some(nat) = l.get("natives").and_then(|n| n.get(env.os.name())).and_then(|c| c.as_str()) {
            let cls = nat.replace("${arch}", if env.arch64 { "64" } else { "32" });
            let art = dl.and_then(|d| d.get("classifiers")).and_then(|c| c.get(&cls));
            let path = art.and_then(|a| a.get("path")).and_then(|p| p.as_str()).map(String::from)
                .or_else(|| maven_path(&format!("{name}:{cls}")));
            if let Some(path) = path {
                out.push(Lib {
                    name: format!("{name}:{cls}"), path,
                    url: art.and_then(|a| a.get("url")).and_then(|u| u.as_str()).map(String::from),
                    sha1: art.and_then(|a| a.get("sha1")).and_then(|u| u.as_str()).map(String::from),
                    native: true, extract_exclude: excl,
                });
            }
            // у такой библиотеки основной артефакт обычно тоже есть (и без нативов) — продолжаем
            if dl.and_then(|d| d.get("artifact")).is_none() { continue; }
        }

        let key = lib_key(name);
        if !seen.insert(key) { continue; }
        let art = dl.and_then(|d| d.get("artifact"));
        let path = art.and_then(|a| a.get("path")).and_then(|p| p.as_str()).map(String::from).or_else(|| maven_path(name));
        let Some(path) = path else { continue };
        let url = art.and_then(|a| a.get("url")).and_then(|u| u.as_str()).filter(|u| !u.is_empty()).map(String::from)
            .or_else(|| l.get("url").and_then(|b| b.as_str()).map(|b| format!("{}/{}", b.trim_end_matches('/'), path)));
        out.push(Lib {
            name: name.to_string(), path,
            url, sha1: art.and_then(|a| a.get("sha1")).and_then(|u| u.as_str()).map(String::from),
            native: false, extract_exclude: vec![],
        });
    }
    out
}

pub struct Params<'a> {
    pub env: Env,
    pub version_name: &'a str,
    pub game_dir: &'a str,
    pub libraries_dir: &'a str,
    pub assets_dir: &'a str,
    pub natives_dir: &'a str,
    pub client_jar: &'a str,
    pub player_name: &'a str,
    pub uuid: &'a str,
    pub access_token: &'a str,
    pub user_type: &'a str,      // "msa" | "mojang" | "legacy"
    pub launcher_name: &'a str,
    pub launcher_version: &'a str,
    pub extra_jvm: Vec<String>,  // пресет/RAM + javaagent
    pub resolution: Option<(u32, u32)>,
}

fn join_path(a: &str, b: &str, env: &Env) -> String {
    let sep = if env.os == Os::Windows { "\\" } else { "/" };
    let b = if env.os == Os::Windows { b.replace('/', "\\") } else { b.to_string() };
    format!("{}{}{}", a.trim_end_matches(['/', '\\']), sep, b)
}

fn expand_args(list: Option<&Value>, env: &Env) -> Vec<String> {
    let mut out = vec![];
    for a in list.and_then(|l| l.as_array()).into_iter().flatten() {
        match a {
            Value::String(s) => out.push(s.clone()),
            Value::Object(_) => {
                if !rules_allow(a.get("rules"), env) { continue; }
                match a.get("value") {
                    Some(Value::String(s)) => out.push(s.clone()),
                    Some(Value::Array(v)) => out.extend(v.iter().filter_map(|x| x.as_str().map(String::from))),
                    _ => {}
                }
            }
            _ => {}
        }
    }
    out
}

fn substitute(s: &str, vars: &[(&str, String)]) -> String {
    let mut r = s.to_string();
    for (k, v) in vars { r = r.replace(&format!("${{{k}}}"), v); }
    r
}

/// Итоговая командная строка (без пути к java): jvm-аргументы, главный класс, игровые аргументы.
pub fn build_args(v: &Value, libs: &[Lib], p: &Params) -> Result<Vec<String>, String> {
    let env = &p.env;
    let main = v.get("mainClass").and_then(|m| m.as_str()).ok_or("в JSON версии нет mainClass")?;

    let mut cp: Vec<String> = libs.iter().filter(|l| !l.native).map(|l| join_path(p.libraries_dir, &l.path, env)).collect();
    cp.push(p.client_jar.to_string());
    let classpath = cp.join(env.os.cp_sep());

    let asset_index = v.pointer("/assetIndex/id").and_then(|x| x.as_str())
        .or_else(|| v.get("assets").and_then(|x| x.as_str())).unwrap_or("legacy");
    // Для старых версий (pre-1.6 / legacy) ассеты лежат в виртуальной папке.
    let assets_root = if asset_index == "legacy" || asset_index == "pre-1.6" {
        join_path(&join_path(p.assets_dir, "virtual", env), asset_index, env)
    } else { p.assets_dir.to_string() };

    let (w, h) = p.resolution.unwrap_or((854, 480));
    let vars: Vec<(&str, String)> = vec![
        ("natives_directory", p.natives_dir.into()), ("launcher_name", p.launcher_name.into()),
        ("launcher_version", p.launcher_version.into()), ("classpath", classpath.clone()),
        ("classpath_separator", env.os.cp_sep().into()), ("library_directory", p.libraries_dir.into()),
        ("auth_player_name", p.player_name.into()), ("version_name", p.version_name.into()),
        ("game_directory", p.game_dir.into()), ("assets_root", assets_root.clone()),
        ("game_assets", assets_root), ("assets_index_name", asset_index.into()),
        ("auth_uuid", p.uuid.into()), ("auth_access_token", p.access_token.into()),
        ("auth_session", format!("token:{}:{}", p.access_token, p.uuid)),
        ("user_type", p.user_type.into()), ("user_properties", "{}".into()),
        ("clientid", String::new()), ("auth_xuid", String::new()),
        ("version_type", v.get("type").and_then(|t| t.as_str()).unwrap_or("release").into()),
        ("resolution_width", w.to_string()), ("resolution_height", h.to_string()),
    ];

    let mut out: Vec<String> = p.extra_jvm.clone();
    let (jvm, game): (Vec<String>, Vec<String>) = if let Some(args) = v.get("arguments") {
        (expand_args(args.get("jvm"), env), expand_args(args.get("game"), env))
    } else {
        // Старый формат: JVM-часть фиксирована, игровые аргументы строкой.
        let jvm = vec![
            format!("-Djava.library.path={}", p.natives_dir), "-cp".into(), classpath.clone(),
        ];
        let game = v.get("minecraftArguments").and_then(|a| a.as_str()).unwrap_or("")
            .split_whitespace().map(String::from).collect();
        (jvm, game)
    };
    // Windows-специфика старых JSON обычно уже в rules; добавляем отсутствующее.
    let mut jvm: Vec<String> = jvm.iter().map(|a| substitute(a, &vars)).collect();
    if !jvm.iter().any(|a| a.starts_with("-Djava.library.path")) {
        jvm.push(format!("-Djava.library.path={}", p.natives_dir));
    }
    if !jvm.iter().any(|a| a == "-cp" || a == "-classpath") {
        jvm.push("-cp".into()); jvm.push(classpath);
    }
    out.extend(jvm);
    out.push(main.to_string());
    let mut game: Vec<String> = game.iter().map(|a| substitute(a, &vars)).collect();
    if p.resolution.is_some() && !game.iter().any(|a| a == "--width") {
        game.extend(["--width".into(), w.to_string(), "--height".into(), h.to_string()]);
    }
    out.extend(game);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params<'a>(extra: Vec<String>) -> Params<'a> {
        Params { env: Env::new(Os::Linux), version_name: "t", game_dir: "/g", libraries_dir: "/L", assets_dir: "/A",
            natives_dir: "/N", client_jar: "/v/t.jar", player_name: "Steve", uuid: "u", access_token: "tok",
            user_type: "legacy", launcher_name: "Redshift", launcher_version: "1", extra_jvm: extra, resolution: None }
    }

    #[test]
    fn maven_paths() {
        assert_eq!(maven_path("org.ow2.asm:asm:9.6").unwrap(), "org/ow2/asm/asm/9.6/asm-9.6.jar");
        assert_eq!(maven_path("a.b:c:1:natives-windows").unwrap(), "a/b/c/1/c-1-natives-windows.jar");
        assert_eq!(maven_path("a:b:1@zip").unwrap(), "a/b/1/b-1.zip");
        assert!(maven_path("bad").is_none());
    }

    #[test]
    fn rules_os_and_features() {
        let env = Env::new(Os::Windows);
        let r = json!([{"action":"allow"},{"action":"deny","os":{"name":"osx"}}]);
        assert!(rules_allow(Some(&r), &env));
        assert!(!rules_allow(Some(&r), &Env::new(Os::Osx)));
        let only_linux = json!([{"action":"allow","os":{"name":"linux"}}]);
        assert!(!rules_allow(Some(&only_linux), &env));
        let demo = json!([{"action":"allow","features":{"is_demo_user":true}}]);
        assert!(!rules_allow(Some(&demo), &env));
        assert!(rules_allow(None, &env));
    }

    #[test]
    fn child_libs_override_parent_versions() {
        // Fabric/Forge приносят свою версию ASM — должна победить, иначе краш NoSuchMethodError.
        let parent = json!({"mainClass":"net.minecraft.client.main.Main","libraries":[{"name":"org.ow2.asm:asm:9.2"},{"name":"x:y:1"}]});
        let child = json!({"mainClass":"net.fabricmc.loader.impl.launch.knot.KnotClient","inheritsFrom":"1.20.1",
            "libraries":[{"name":"org.ow2.asm:asm:9.6","url":"https://maven.fabricmc.net/"}]});
        let m = merge(&parent, &child);
        let libs = libraries(&m, &Env::new(Os::Linux));
        let asm: Vec<_> = libs.iter().filter(|l| l.name.contains("asm")).collect();
        assert_eq!(asm.len(), 1);
        assert_eq!(asm[0].name, "org.ow2.asm:asm:9.6");
        assert_eq!(asm[0].url.as_deref(), Some("https://maven.fabricmc.net/org/ow2/asm/asm/9.6/asm-9.6.jar"));
        assert_eq!(m["mainClass"], "net.fabricmc.loader.impl.launch.knot.KnotClient");
    }

    #[test]
    fn modern_arguments() {
        let v = json!({"mainClass":"M","assetIndex":{"id":"12"},"type":"release","arguments":{
            "game":["--username","${auth_player_name}",{"rules":[{"action":"allow","features":{"is_demo_user":true}}],"value":"--demo"}],
            "jvm":["-Djava.library.path=${natives_directory}",{"rules":[{"action":"allow","os":{"name":"windows"}}],"value":"-XX:HeapDumpPath=x"},"-cp","${classpath}"]},
            "libraries":[{"name":"a:b:1","downloads":{"artifact":{"path":"a/b/1/b-1.jar","url":"u","sha1":"s"}}}]});
        let libs = libraries(&v, &Env::new(Os::Linux));
        let a = build_args(&v, &libs, &params(vec!["-Xmx2G".into()])).unwrap();
        assert_eq!(a[0], "-Xmx2G");
        assert!(a.contains(&"-Djava.library.path=/N".to_string()));
        assert!(!a.contains(&"-XX:HeapDumpPath=x".to_string()));
        assert!(!a.contains(&"--demo".to_string()));
        let cp = &a[a.iter().position(|x| x == "-cp").unwrap() + 1];
        assert_eq!(cp, "/L/a/b/1/b-1.jar:/v/t.jar");
        let mi = a.iter().position(|x| x == "M").unwrap();
        assert_eq!(&a[mi + 1..], ["--username", "Steve"]);
    }

    #[test]
    fn legacy_arguments_and_virtual_assets() {
        let v = json!({"mainClass":"net.minecraft.launchwrapper.Launch","assets":"legacy","type":"release",
            "minecraftArguments":"--username ${auth_player_name} --session ${auth_session} --gameDir ${game_directory} --assetsDir ${game_assets}",
            "libraries":[]});
        let a = build_args(&v, &[], &params(vec![])).unwrap();
        assert!(a.contains(&"-cp".to_string()), "classpath должен добавляться");
        assert!(a.contains(&"-Djava.library.path=/N".to_string()));
        assert!(a.contains(&"token:tok:u".to_string()));
        assert!(a.contains(&"/A/virtual/legacy".to_string()));
    }

    #[test]
    fn old_style_natives() {
        let v = json!({"libraries":[{"name":"org.lwjgl.lwjgl:lwjgl-platform:2.9.4","natives":{"linux":"natives-linux","windows":"natives-windows"},
            "extract":{"exclude":["META-INF/"]},
            "downloads":{"classifiers":{"natives-windows":{"path":"o/lw-natives-windows.jar","url":"w","sha1":"1"},"natives-linux":{"path":"o/lw-natives-linux.jar","url":"l","sha1":"2"}}}}]});
        let l = libraries(&v, &Env::new(Os::Windows));
        assert_eq!(l.len(), 1);
        assert!(l[0].native);
        assert_eq!(l[0].path, "o/lw-natives-windows.jar");
        assert_eq!(l[0].extract_exclude, vec!["META-INF/"]);
    }

    #[test]
    fn missing_main_class_errors() {
        assert!(build_args(&json!({}), &[], &params(vec![])).is_err());
    }

    #[test]
    fn windows_paths() {
        let mut p = params(vec![]);
        p.env = Env::new(Os::Windows);
        p.libraries_dir = "C:\\L";
        let v = json!({"mainClass":"M","libraries":[{"name":"a:b:1"}]});
        let libs = libraries(&v, &p.env);
        let a = build_args(&v, &libs, &p).unwrap();
        let cp = &a[a.iter().position(|x| x == "-cp").unwrap() + 1];
        assert_eq!(cp, "C:\\L\\a\\b\\1\\b-1.jar;/v/t.jar");
    }
}

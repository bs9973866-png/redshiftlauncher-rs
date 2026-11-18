//! Оркестрация запуска: установка → Java → аргументы → процесс → диагностика и безопасный автоповтор.

use crate::diag::{self, Fix};
use crate::install::{self, Dirs};
use crate::jvm::{self, Preset};
use crate::launch::{self, Env, Os, Params};
use crate::loaders::{self, Loader};
use crate::net::{self, Result};
use crate::runner::Outcome;
use crate::settings::{self, Account, Instance, Settings};
use crate::{accounts, java, javart};
use std::fs;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq)]
pub struct Attempt { pub preset: Preset, pub ram_mb: u32 }

pub struct Sys { pub ram_mb: u32, pub cpus: usize }

pub fn system() -> Sys {
    let cpus = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2);
    Sys { ram_mb: total_ram_mb().unwrap_or(8192), cpus }
}

#[cfg(target_os = "linux")]
fn total_ram_mb() -> Option<u32> {
    let s = fs::read_to_string("/proc/meminfo").ok()?;
    let kb: u64 = s.lines().find(|l| l.starts_with("MemTotal:"))?.split_whitespace().nth(1)?.parse().ok()?;
    Some((kb / 1024) as u32)
}
#[cfg(windows)]
fn total_ram_mb() -> Option<u32> {
    #[repr(C)] struct MemStatus { len: u32, load: u32, total_phys: u64, avail_phys: u64, tp: u64, ap: u64, tv: u64, av: u64, ext: u64 }
    extern "system" { fn GlobalMemoryStatusEx(s: *mut MemStatus) -> i32; }
    let mut m = MemStatus { len: std::mem::size_of::<MemStatus>() as u32, load: 0, total_phys: 0, avail_phys: 0, tp: 0, ap: 0, tv: 0, av: 0, ext: 0 };
    if unsafe { GlobalMemoryStatusEx(&mut m) } == 0 { return None; }
    Some((m.total_phys / 1024 / 1024) as u32)
}
#[cfg(not(any(target_os = "linux", windows)))]
fn total_ram_mb() -> Option<u32> { None }

/// Первая попытка: настройки пользователя + защита от слишком большого RAM.
pub fn first_attempt(s: &Settings, inst: &Instance, sys: &Sys) -> Attempt {
    let wanted = if inst.ram_mb > 0 { inst.ram_mb } else { s.ram_mb };
    let preset = if s.jvm_preset == "auto" { jvm::auto_preset(sys.ram_mb, sys.cpus) } else { Preset::parse(&s.jvm_preset) };
    Attempt { preset, ram_mb: jvm::clamp_ram_mb(wanted, sys.ram_mb, inst.heavy || inst.loader == "forge" || inst.loader == "neoforge") }
}

/// Запускает `exec`, при известных сбоях JVM пробует ещё раз (не более 2 повторов).
/// Возвращает последний результат и список сообщений для пользователя.
pub fn run_with_retry(first: Attempt, mut exec: impl FnMut(&Attempt) -> Result<Outcome>, notify: &dyn Fn(&str)) -> Result<(Outcome, Vec<String>)> {
    let mut a = first;
    let (mut safe_done, mut ram_done) = (a.preset == Preset::Safe, false);
    let mut notes = vec![];
    for _ in 0..3 {
        let out = exec(&a)?;
        if out.code == Some(0) { return Ok((out, notes)); }
        let d = diag::analyze(&out.tail);
        notes.push(d.message.to_string()).clone_from(&());
        if d.message.is_empty() { notes.pop(); }
        if !diag::should_retry(&d, safe_done, ram_done) { return Ok((out, notes)); }
        notify(d.message);
        match d.fix {
            Fix::RetrySafeJvm => { a.preset = Preset::Safe; safe_done = true; }
            Fix::LowerRam => { a.ram_mb = (a.ram_mb * 2 / 3).max(1024); a.preset = Preset::Safe; ram_done = true; safe_done = true; }
            _ => return Ok((out, notes)),
        }
    }
    Err("слишком много неудачных попыток запуска".into())
}

pub struct Ctx<'a> {
    pub dirs: Dirs,
    pub store: &'a settings::Store,
    pub agent: ureq::Agent,
    pub sys: Sys,
}

fn env() -> Env { Env::new(Os::current()) }

/// Убеждается, что ванильная версия `mc` лежит на диске (JSON), и возвращает её.
pub fn ensure_vanilla_json(c: &Ctx, mc: &str) -> Result<()> {
    if c.dirs.version_json(mc).is_file() { return Ok(()); }
    let m = net::get_json(&c.agent, install::MANIFEST_URL)?;
    let e = install::parse_manifest(&m).into_iter().find(|e| e.id == mc).ok_or_else(|| format!("версия {mc} не найдена в манифесте Mojang"))?;
    net::download_one(&c.agent, &net::Job { url: e.url, dest: c.dirs.version_json(mc), sha1: None })
}

/// Установка загрузчика сборки; возвращает id версии для запуска.
pub fn ensure_loader(c: &Ctx, inst: &Instance, progress: &(dyn Fn(&str) + Sync)) -> Result<String> {
    let l = Loader::parse(&inst.loader);
    ensure_vanilla_json(c, &inst.mc)?;
    if l == Loader::Vanilla { return Ok(inst.mc.clone()); }
    if let Some(id) = &inst.launch_id { if c.dirs.version_json(id).is_file() { return Ok(id.clone()); } }
    progress(&format!("Установка {}…", l.id()));
    match l {
        Loader::Fabric | Loader::Quilt => loaders::install_profile_loader(&c.agent, &c.dirs, l, &inst.mc, inst.loader_version.as_deref()),
        _ => {
            if let Some(id) = loaders::find_existing(&c.dirs, l, &inst.mc) { return Ok(id); }
            // установщик Forge: Java берём по версии игры (для старых Forge — строго 8)
            let base = ensure_vanilla_and_files(c, &inst.mc, progress)?;
            let major = java::adjust_for_loader(base.1, l.id(), &inst.mc);
            let j = javart::ensure(&c.agent, &c.dirs, major, cfg!(windows))?;
            loaders::install_installer_loader(&c.agent, &c.dirs, &j, l, &inst.mc, &c.dirs.root.join("tmp"), &|_| {})
        }
    }
}

fn ensure_vanilla_and_files(c: &Ctx, mc: &str, progress: &(dyn Fn(&str) + Sync)) -> Result<(serde_json::Value, u32)> {
    ensure_vanilla_json(c, mc)?;
    let v = install::ensure_version_files(&c.agent, &c.dirs, mc, &env(), c.store.settings.download_threads as usize,
        &|what, d, t| progress(&format!("{what}: {d}/{t}")))?;
    let major = java::required_java(v.pointer("/javaVersion/majorVersion").and_then(|m| m.as_u64()).map(|m| m as u32), mc,
        v.get("type").and_then(|t| t.as_str()).unwrap_or("release"));
    Ok((v, major))
}

/// Полный запуск. Блокирует поток до закрытия игры — вызывать из рабочего потока.
pub fn play(c: &Ctx, inst: &Instance, acc: &Account, progress: &(dyn Fn(&str) + Sync), log: &(dyn Fn(&str) + Sync)) -> Result<Outcome> {
    let launch_id = ensure_loader(c, inst, progress)?;
    let v = install::ensure_version_files(&c.agent, &c.dirs, &launch_id, &env(), c.store.settings.download_threads as usize,
        &|what, d, t| progress(&format!("{what}: {d}/{t}")))?;
    let l = Loader::parse(&inst.loader);
    let declared = v.pointer("/javaVersion/majorVersion").and_then(|m| m.as_u64()).map(|m| m as u32);
    let kind = v.get("type").and_then(|t| t.as_str()).unwrap_or("release");
    let major = java::adjust_for_loader(java::required_java(declared, &inst.mc, kind), l.id(), &inst.mc);
    progress(&format!("Java {major}…"));
    let java_exe = javart::ensure(&c.agent, &c.dirs, major, cfg!(windows))?;

    let libs = launch::libraries(&v, &env());
    let game_dir = settings::instance_dir(&c.dirs.root, &inst.id);
    fs::create_dir_all(&game_dir).map_err(|e| e.to_string())?;
    let natives = c.dirs.version_dir(&launch_id).join("natives");
    let _ = fs::remove_dir_all(&natives);
    install::extract_natives(&c.dirs, &libs, &natives)?;
    if let Some(idx) = v.pointer("/assetIndex/id").and_then(|i| i.as_str()) {
        let ip = c.dirs.assets().join("indexes").join(format!("{idx}.json"));
        if let Ok(s) = fs::read_to_string(&ip) { if let Ok(j) = serde_json::from_str(&s) { install::materialize_virtual(&c.dirs, &j, idx, &game_dir)?; } }
    }

    let mut agent_arg = None;
    if acc.kind == "ely" {
        let jar = c.dirs.root.join("authlib-injector.jar");
        if !jar.is_file() {
            let url = accounts::authlib_injector_url(&c.agent)?;
            net::download_one(&c.agent, &net::Job { url, dest: jar.clone(), sha1: None })?;
        }
        agent_arg = Some(accounts::ely_agent_arg(&jar));
    }

    let client_jar = install::resolved_client_jar(&c.dirs, &v, &launch_id);
    let first = first_attempt(&c.store.settings, inst, &c.sys);
    let token = acc.access_token.clone().unwrap_or_else(|| "0".into());
    let user_type = if acc.kind == "offline" { "legacy" } else { "mojang" };
    let (dirs_l, dirs_a, nat, gd, cj) = (c.dirs.libraries(), c.dirs.assets(), natives.clone(), game_dir.clone(), client_jar.clone());
    let s = |p: &PathBuf| p.to_string_lossy().to_string();
    let (dl, da, dn, dg, dc) = (s(&dirs_l), s(&dirs_a), s(&nat), s(&gd), s(&cj));

    let (out, _) = run_with_retry(first, |a| {
        let mut extra = jvm::args(a.preset, a.ram_mb, major);
        if let Some(ag) = &agent_arg { extra.insert(0, ag.clone()); }
        let args = launch::build_args(&v, &libs, &Params {
            env: env(), version_name: &launch_id, game_dir: &dg, libraries_dir: &dl, assets_dir: &da, natives_dir: &dn,
            client_jar: &dc, player_name: &acc.name, uuid: &acc.uuid, access_token: &token, user_type,
            launcher_name: "Redshift", launcher_version: env!("CARGO_PKG_VERSION"), extra_jvm: extra, resolution: None })?;
        progress(&format!("Запуск ({:?}, {} МБ)…", a.preset, a.ram_mb));
        crate::runner::run(&java_exe.to_string_lossy(), &args, &game_dir, log)
    }, &|m| log(&format!("[Redshift] {m}")))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn out(code: i32, tail: &str) -> Outcome { Outcome { code: Some(code), tail: tail.into() } }

    #[test]
    fn retries_with_safe_flags_then_succeeds() {
        let seen = RefCell::new(vec![]);
        let (o, notes) = run_with_retry(Attempt { preset: Preset::Balanced, ram_mb: 4096 }, |a| {
            seen.borrow_mut().push(a.clone());
            if a.preset == Preset::Safe { Ok(out(0, "")) } else { Ok(out(1, "Unrecognized VM option 'X'")) }
        }, &|_| {}).unwrap();
        assert_eq!(o.code, Some(0));
        assert_eq!(seen.borrow().len(), 2);
        assert_eq!(seen.borrow()[1], Attempt { preset: Preset::Safe, ram_mb: 4096 });
        assert_eq!(notes.len(), 1);
    }

    #[test]
    fn lowers_ram_on_reserve_error() {
        let seen = RefCell::new(vec![]);
        let (o, _) = run_with_retry(Attempt { preset: Preset::Max, ram_mb: 6000 }, |a| {
            seen.borrow_mut().push(a.ram_mb);
            if a.ram_mb <= 4000 { Ok(out(0, "")) } else { Ok(out(1, "Could not reserve enough space for object heap")) }
        }, &|_| {}).unwrap();
        assert_eq!(o.code, Some(0));
        assert_eq!(*seen.borrow(), vec![6000, 4000]);
    }

    #[test]
    fn does_not_loop_forever() {
        let n = RefCell::new(0);
        let (o, _) = run_with_retry(Attempt { preset: Preset::Balanced, ram_mb: 2048 }, |_| {
            *n.borrow_mut() += 1; Ok(out(1, "Unrecognized VM option 'Y'"))
        }, &|_| {}).unwrap();
        assert_eq!(o.code, Some(1));
        assert_eq!(*n.borrow(), 2, "один повтор с Safe, потом стоп");
    }

    #[test]
    fn game_crash_is_not_retried() {
        let n = RefCell::new(0);
        let (o, notes) = run_with_retry(Attempt { preset: Preset::Balanced, ram_mb: 2048 }, |_| { *n.borrow_mut() += 1; Ok(out(1, "Exception in mod xyz")) }, &|_| {}).unwrap();
        assert_eq!((o.code, *n.borrow(), notes.len()), (Some(1), 1, 0));
    }

    #[test]
    fn weak_pc_first_attempt_is_conservative() {
        let s = Settings { ram_mb: 4096, ..Default::default() };
        let inst = Instance { loader: "vanilla".into(), ..Default::default() };
        let a = first_attempt(&s, &inst, &Sys { ram_mb: 4096, cpus: 2 });
        assert_eq!(a.preset, Preset::LowEnd);
        assert!(a.ram_mb <= 1536);
    }

    #[test]
    fn system_info_sane() {
        let s = system();
        assert!(s.cpus >= 1 && s.ram_mb >= 256);
    }
}

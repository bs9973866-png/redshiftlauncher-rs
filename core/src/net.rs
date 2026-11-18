//! Лёгкая сетевая часть: blocking ureq + ограниченный пул потоков (без tokio).
//! Скачивание атомарное (во временный файл + rename), с проверкой sha1 и повторами.

use sha1::{Digest, Sha1};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub type Result<T> = std::result::Result<T, String>;

pub fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(15))
        .timeout_read(Duration::from_secs(60))
        .user_agent("RedshiftLauncher/0.1 (+https://redshift)")
        .build()
}

pub fn get_json(agent: &ureq::Agent, url: &str) -> Result<serde_json::Value> {
    agent.get(url).call().map_err(|e| format!("{url}: {e}"))?
        .into_json().map_err(|e| format!("{url}: некорректный JSON: {e}"))
}

pub fn sha1_file(p: &Path) -> Option<String> {
    let mut f = fs::File::open(p).ok()?;
    let mut h = Sha1::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 { break; }
        h.update(&buf[..n]);
    }
    Some(hex::encode(h.finalize()))
}

#[derive(Clone, Debug)]
pub struct Job { pub url: String, pub dest: PathBuf, pub sha1: Option<String> }

/// true — файл уже на месте и корректен (если sha1 известен).
pub fn is_ok(job: &Job) -> bool {
    match (job.dest.is_file(), &job.sha1) {
        (true, Some(s)) => sha1_file(&job.dest).as_deref() == Some(&s.to_lowercase()),
        (true, None) => fs::metadata(&job.dest).map(|m| m.len() > 0).unwrap_or(false),
        _ => false,
    }
}

pub fn download_one(agent: &ureq::Agent, job: &Job) -> Result<()> {
    if is_ok(job) { return Ok(()); }
    if let Some(dir) = job.dest.parent() { fs::create_dir_all(dir).map_err(|e| e.to_string())?; }
    let mut last = String::new();
    for attempt in 0..3 {
        if attempt > 0 { std::thread::sleep(Duration::from_millis(400 * attempt as u64)); }
        let tmp = job.dest.with_extension("part");
        let res = (|| -> Result<()> {
            let resp = agent.get(&job.url).call().map_err(|e| format!("{}: {e}", job.url))?;
            let mut r = resp.into_reader();
            let mut f = fs::File::create(&tmp).map_err(|e| e.to_string())?;
            let mut h = Sha1::new();
            let mut buf = [0u8; 64 * 1024];
            loop {
                let n = r.read(&mut buf).map_err(|e| e.to_string())?;
                if n == 0 { break; }
                h.update(&buf[..n]);
                f.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            }
            drop(f);
            if let Some(want) = &job.sha1 {
                let got = hex::encode(h.finalize());
                if !got.eq_ignore_ascii_case(want) {
                    let _ = fs::remove_file(&tmp);
                    return Err(format!("{}: sha1 не совпал (ожидали {want}, получили {got})", job.url));
                }
            }
            fs::rename(&tmp, &job.dest).map_err(|e| e.to_string())
        })();
        match res { Ok(()) => return Ok(()), Err(e) => { last = e; let _ = fs::remove_file(&tmp); } }
    }
    Err(last)
}

/// Параллельная загрузка. `threads` держим малым (по умолчанию 4) — это и есть "лёгкость" на слабых ПК.
/// `progress(done, total)` вызывается не чаще ~10 раз/сек.
pub fn download_all(agent: &ureq::Agent, jobs: Vec<Job>, threads: usize, progress: &(dyn Fn(usize, usize) + Sync)) -> Result<()> {
    let total = jobs.len();
    let queue = Arc::new(Mutex::new(jobs));
    let done = Arc::new(AtomicUsize::new(0));
    let err: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let last_tick = Arc::new(Mutex::new(std::time::Instant::now() - Duration::from_secs(1)));
    std::thread::scope(|s| {
        for _ in 0..threads.clamp(1, 16) {
            let (queue, done, err, last_tick) = (queue.clone(), done.clone(), err.clone(), last_tick.clone());
            s.spawn(move || loop {
                if err.lock().unwrap().is_some() { return; }
                let Some(job) = queue.lock().unwrap().pop() else { return };
                match download_one(agent, &job) {
                    Ok(()) => {
                        let d = done.fetch_add(1, Ordering::SeqCst) + 1;
                        let mut t = last_tick.lock().unwrap();
                        if d == total || t.elapsed() >= Duration::from_millis(100) { *t = std::time::Instant::now(); progress(d, total); }
                    }
                    Err(e) => { *err.lock().unwrap() = Some(e); return; }
                }
            });
        }
    });
    let e = err.lock().unwrap().take();
    match e { Some(e) => Err(e), None => Ok(()) }
}

#[cfg(test)]
pub mod testsrv {
    use std::collections::HashMap;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// Мини HTTP-сервер для тестов: путь -> тело. Возвращает базовый URL.
    pub fn serve(files: HashMap<String, Vec<u8>>) -> String {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for s in l.incoming() {
                let Ok(mut s) = s else { continue };
                let files = files.clone();
                std::thread::spawn(move || {
                    let mut buf = [0u8; 4096];
                    let n = s.read(&mut buf).unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]).to_string();
                    let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
                    match files.get(&path) {
                        Some(b) => {
                            let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", b.len());
                            let _ = s.write_all(b);
                        }
                        None => { let _ = write!(s, "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"); }
                    }
                });
            }
        });
        format!("http://127.0.0.1:{port}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn sha(b: &[u8]) -> String { hex::encode(Sha1::digest(b)) }
    fn tmpdir(n: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rs-test-{n}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d); fs::create_dir_all(&d).unwrap(); d
    }

    #[test]
    fn downloads_and_verifies() {
        let mut f = HashMap::new();
        f.insert("/a.jar".to_string(), b"hello".to_vec());
        let base = testsrv::serve(f);
        let d = tmpdir("dl");
        let ag = agent();
        let ok = Job { url: format!("{base}/a.jar"), dest: d.join("x/a.jar"), sha1: Some(sha(b"hello")) };
        download_one(&ag, &ok).unwrap();
        assert_eq!(fs::read(&ok.dest).unwrap(), b"hello");
        assert!(is_ok(&ok));
        let bad = Job { url: format!("{base}/a.jar"), dest: d.join("b.jar"), sha1: Some(sha(b"other")) };
        let e = download_one(&ag, &bad).unwrap_err();
        assert!(e.contains("sha1"), "{e}");
        assert!(!bad.dest.exists(), "битый файл не должен остаться");
        let missing = Job { url: format!("{base}/nope"), dest: d.join("c.jar"), sha1: None };
        assert!(download_one(&ag, &missing).is_err());
    }

    #[test]
    fn parallel_with_progress_and_skip_existing() {
        let mut f = HashMap::new();
        let mut jobs = vec![];
        let d = tmpdir("par");
        for i in 0..20 { f.insert(format!("/f{i}"), format!("data{i}").into_bytes()); }
        let base = testsrv::serve(f);
        for i in 0..20 { jobs.push(Job { url: format!("{base}/f{i}"), dest: d.join(format!("f{i}")), sha1: Some(sha(format!("data{i}").as_bytes())) }); }
        let calls = AtomicUsize::new(0);
        download_all(&agent(), jobs.clone(), 4, &|_, _| { calls.fetch_add(1, Ordering::SeqCst); }).unwrap();
        assert!(calls.load(Ordering::SeqCst) >= 1);
        for j in &jobs { assert!(is_ok(j)); }
        // второй запуск: сервер не нужен (файлы уже есть)
        let jobs2: Vec<Job> = jobs.iter().map(|j| Job { url: "http://127.0.0.1:1/x".into(), ..j.clone() }).collect();
        download_all(&agent(), jobs2, 4, &|_, _| {}).unwrap();
    }

    #[test]
    fn error_stops_batch() {
        let d = tmpdir("err");
        let jobs = vec![Job { url: "http://127.0.0.1:1/x".into(), dest: d.join("z"), sha1: None }];
        assert!(download_all(&agent(), jobs, 2, &|_, _| {}).is_err());
    }
}

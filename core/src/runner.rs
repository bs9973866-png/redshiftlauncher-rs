//! Запуск процесса игры: чтение вывода с ограниченным буфером (не раздувает память и не грузит UI).

use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

pub const TAIL_LINES: usize = 400;

pub struct Outcome { pub code: Option<i32>, pub tail: String }

pub fn run(program: &str, args: &[String], cwd: &std::path::Path, on_line: &(dyn Fn(&str) + Sync)) -> Result<Outcome, String> {
    let mut cmd = Command::new(program);
    cmd.args(args).current_dir(cwd).stdout(Stdio::piped()).stderr(Stdio::piped()).stdin(Stdio::null());
    #[cfg(windows)]
    { use std::os::windows::process::CommandExt; cmd.creation_flags(0x0800_0000); } // CREATE_NO_WINDOW
    let mut child = cmd.spawn().map_err(|e| format!("не удалось запустить {program}: {e}"))?;
    let tail: Arc<Mutex<VecDeque<String>>> = Arc::new(Mutex::new(VecDeque::with_capacity(TAIL_LINES)));
    let out = child.stdout.take().unwrap();
    let err = child.stderr.take().unwrap();
    std::thread::scope(|s| {
        for r in [Box::new(out) as Box<dyn std::io::Read + Send>, Box::new(err)] {
            let tail = tail.clone();
            s.spawn(move || {
                for line in BufReader::new(r).split(b'\n').map_while(Result::ok) {
                    let line = String::from_utf8_lossy(&line).trim_end_matches('\r').to_string();
                    on_line(&line);
                    let mut t = tail.lock().unwrap();
                    if t.len() == TAIL_LINES { t.pop_front(); }
                    t.push_back(line);
                }
            });
        }
    });
    let status = child.wait().map_err(|e| e.to_string())?;
    let tail = tail.lock().unwrap().iter().cloned().collect::<Vec<_>>().join("\n");
    Ok(Outcome { code: status.code(), tail })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn captures_both_streams_and_code() {
        let seen = Mutex::new(vec![]);
        let o = run("sh", &["-c".into(), "echo out; echo err 1>&2; exit 3".into()], std::path::Path::new("/"), &|l| seen.lock().unwrap().push(l.to_string())).unwrap();
        assert_eq!(o.code, Some(3));
        assert!(o.tail.contains("out") && o.tail.contains("err"));
        assert_eq!(seen.lock().unwrap().len(), 2);
    }
    #[test]
    fn tail_is_bounded() {
        let o = run("sh", &["-c".into(), "i=0; while [ $i -lt 1000 ]; do echo line$i; i=$((i+1)); done".into()], std::path::Path::new("/"), &|_| {}).unwrap();
        assert_eq!(o.tail.lines().count(), TAIL_LINES);
        assert!(o.tail.ends_with("line999"));
    }
    #[test]
    fn missing_program_is_error() {
        assert!(run("definitely-not-a-program-xyz", &[], std::path::Path::new("/"), &|_| {}).is_err());
    }
}

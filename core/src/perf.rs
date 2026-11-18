//! «Оптимизировать FPS»: настройки видео в options.txt + список FPS-модов (порт perf.js).

use std::fs;
use std::path::Path;

pub fn fps_mods(loader: &str) -> &'static [&'static str] {
    match loader {
        "fabric" => &["fabric-api", "sodium", "lithium", "ferrite-core", "entityculling", "immediatelyfast", "modernfix", "krypton", "dynamic-fps"],
        "quilt" => &["qsl", "sodium", "lithium", "ferrite-core", "entityculling", "immediatelyfast", "modernfix", "krypton"],
        "forge" | "neoforge" => &["embeddium", "modernfix", "ferrite-core", "entityculling"],
        _ => &[],
    }
}

fn tweaks(extreme: bool) -> Vec<(&'static str, &'static str)> {
    let mut v = vec![("renderDistance", "8"), ("simulationDistance", "8"), ("entityDistanceScaling", "0.75"), ("particles", "2"),
        ("entityShadows", "false"), ("biomeBlendRadius", "1"), ("mipmapLevels", "2"), ("enableVsync", "false"), ("maxFps", "260"),
        ("graphicsMode", "0"), ("fancyGraphics", "false"), ("renderClouds", "false"), ("ao", "false"), ("bobView", "false"), ("attackIndicator", "0")];
    if extreme {
        for (k, val) in [("renderDistance", "6"), ("simulationDistance", "5"), ("biomeBlendRadius", "0"), ("mipmapLevels", "0")] {
            if let Some(e) = v.iter_mut().find(|e| e.0 == k) { e.1 = val; }
        }
    }
    v
}

/// Объединяет настройки с options.txt. В существующем файле трогаются только ключи, которые игра уже записала
/// (плюс универсальные), в новом — пишутся все. Порядок строк сохраняется.
pub fn apply_video_tweaks(game_dir: &Path, extreme: bool) -> Result<(), String> {
    fs::create_dir_all(game_dir).map_err(|e| e.to_string())?;
    let file = game_dir.join("options.txt");
    let text = fs::read_to_string(&file).unwrap_or_default();
    let mut lines: Vec<(String, String)> = text.lines().filter(|l| !l.is_empty())
        .map(|l| match l.split_once(':') { Some((k, v)) => (k.to_string(), v.to_string()), None => (l.to_string(), String::new()) }).collect();
    let fresh = lines.is_empty();
    for (k, v) in tweaks(extreme) {
        match lines.iter_mut().find(|e| e.0 == k) {
            Some(e) => e.1 = v.to_string(),
            None if fresh || ["renderDistance", "maxFps", "enableVsync", "particles"].contains(&k) => lines.push((k.to_string(), v.to_string())),
            None => {}
        }
    }
    let out: String = lines.iter().map(|(k, v)| format!("{k}:{v}\n")).collect();
    fs::write(file, out).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tmp(n: &str) -> std::path::PathBuf { let d = std::env::temp_dir().join(format!("rs-perf-{n}-{}", std::process::id())); let _ = fs::remove_dir_all(&d); d }

    #[test]
    fn fresh_file_gets_everything() {
        let d = tmp("f"); apply_video_tweaks(&d, false).unwrap();
        let s = fs::read_to_string(d.join("options.txt")).unwrap();
        assert!(s.contains("renderDistance:8\n") && s.contains("ao:false\n") && s.contains("maxFps:260\n"));
    }
    #[test]
    fn existing_file_keeps_unknown_keys_and_order() {
        let d = tmp("e"); fs::create_dir_all(&d).unwrap();
        fs::write(d.join("options.txt"), "version:3465\nlang:ru_ru\nrenderDistance:16\nao:true\n").unwrap();
        apply_video_tweaks(&d, true).unwrap();
        let s = fs::read_to_string(d.join("options.txt")).unwrap();
        assert!(s.starts_with("version:3465\nlang:ru_ru\n"));
        assert!(s.contains("renderDistance:6") && s.contains("ao:false"));
        assert!(!s.contains("fancyGraphics"), "чужие ключи не добавляются в существующий файл");
        assert!(s.contains("maxFps:260"));
    }
    #[test]
    fn mods_per_loader() {
        assert!(fps_mods("fabric").contains(&"sodium") && fps_mods("forge").contains(&"embeddium") && fps_mods("vanilla").is_empty());
    }
}

//! JVM-пресеты, ограничение RAM и безопасный набор флагов.
//!
//! Старые причины падений: слишком много RAM на слабом ПК ("Could not reserve
//! enough space"), флаги, неизвестные старой Java ("Unrecognized VM option"),
//! тяжёлые флаги (AlwaysPreTouch) на слабом железе.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    /// Только -Xmx/-Xms. Используется для автоповтора после сбоя.
    Safe,
    Balanced,
    Max,
    LowEnd,
}

impl Preset {
    pub fn parse(s: &str) -> Preset {
        match s {
            "safe" | "none" => Preset::Safe,
            "max" => Preset::Max,
            "lowend" => Preset::LowEnd,
            _ => Preset::Balanced,
        }
    }
}

/// Разумная RAM в МБ по физической памяти ПК и пожеланию пользователя.
/// Оставляем системе минимум ~2.5 ГБ, не больше 75% физической памяти.
pub fn clamp_ram_mb(wanted: u32, system_mb: u32, heavy_pack: bool) -> u32 {
    let sys = system_mb.max(2048);
    let cap = ((sys as f32 * 0.75) as u32).min(sys.saturating_sub(2560)).max(1024);
    let default = if heavy_pack { 6144 } else { 3072 };
    let w = if wanted == 0 { default } else { wanted };
    w.clamp(1024, cap).min(32768)
}

/// Автовыбор пресета по железу: слабым ПК — LowEnd.
pub fn auto_preset(system_mb: u32, cpus: usize) -> Preset {
    if system_mb <= 6144 || cpus <= 2 { Preset::LowEnd } else { Preset::Balanced }
}

/// Формирует JVM-аргументы. `java_major` нужен, чтобы не передавать флаги,
/// которых нет в этой версии JVM.
pub fn args(preset: Preset, ram_mb: u32, java_major: u32) -> Vec<String> {
    let mut v = vec![format!("-Xmx{ram_mb}M")];
    let min = match preset {
        Preset::Safe | Preset::LowEnd => (ram_mb / 4).max(256),
        Preset::Balanced => (ram_mb / 2).max(512),
        Preset::Max => ram_mb,
    };
    v.push(format!("-Xms{}M", min.min(ram_mb)));
    if preset == Preset::Safe {
        return v;
    }
    v.extend(["-XX:+UseG1GC", "-XX:+UnlockExperimentalVMOptions", "-XX:+DisableExplicitGC"].map(String::from));
    match preset {
        Preset::LowEnd => {
            v.push("-XX:MaxGCPauseMillis=100".into());
            v.push("-XX:ParallelGCThreads=2".into());
            v.push("-XX:ConcGCThreads=1".into());
        }
        Preset::Balanced | Preset::Max => {
            v.extend(["-XX:MaxGCPauseMillis=200", "-XX:G1NewSizePercent=30", "-XX:G1MaxNewSizePercent=40",
                      "-XX:G1HeapRegionSize=8M", "-XX:G1ReservePercent=20", "-XX:InitiatingHeapOccupancyPercent=15"]
                .map(String::from));
            if preset == Preset::Max && java_major >= 11 {
                // AlwaysPreTouch только на Max и только если RAM заведомо хватает (проверяется вызывающим).
                v.push("-XX:+AlwaysPreTouch".into());
            }
        }
        Preset::Safe => unreachable!(),
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ram_never_exceeds_system() {
        // Слабый ПК друга: 4 ГБ, пользователь просит 4096.
        let r = clamp_ram_mb(4096, 4096, false);
        assert!(r <= 1536, "{r}");
        assert!(clamp_ram_mb(16000, 16384, true) <= 12288);
        assert_eq!(clamp_ram_mb(0, 16384, true), 6144);
        assert_eq!(clamp_ram_mb(0, 16384, false), 3072);
        assert!(clamp_ram_mb(1, 8192, false) >= 1024);
    }

    #[test]
    fn safe_has_only_heap_flags() {
        let a = args(Preset::Safe, 2048, 8);
        assert!(a.iter().all(|x| x.starts_with("-Xm")), "{a:?}");
    }

    #[test]
    fn pretouch_only_on_max_modern() {
        assert!(args(Preset::Max, 4096, 17).contains(&"-XX:+AlwaysPreTouch".to_string()));
        assert!(!args(Preset::Max, 4096, 8).contains(&"-XX:+AlwaysPreTouch".to_string()));
        assert!(!args(Preset::Balanced, 4096, 17).contains(&"-XX:+AlwaysPreTouch".to_string()));
    }

    #[test]
    fn weak_pc_gets_lowend() {
        assert_eq!(auto_preset(4096, 4), Preset::LowEnd);
        assert_eq!(auto_preset(16384, 2), Preset::LowEnd);
        assert_eq!(auto_preset(16384, 8), Preset::Balanced);
    }

    #[test]
    fn xms_not_above_xmx() {
        for p in [Preset::Safe, Preset::Balanced, Preset::Max, Preset::LowEnd] {
            let a = args(p, 1024, 17);
            let xms: u32 = a[1].trim_start_matches("-Xms").trim_end_matches('M').parse().unwrap();
            assert!(xms <= 1024);
        }
    }
}

//! Выбор версии Java для запуска Minecraft.
//!
//! Главная причина "старые версии / Forge не запускаются" в прежнем лаунчере —
//! неверная Java. Здесь приоритет такой:
//!   1. `javaVersion.majorVersion` из JSON версии Mojang (если есть);
//!   2. эвристика по id версии (release / snapshot / old_beta / old_alpha);
//! и результат округляется до LTS-сборки Temurin: 8, 17, 21, 25.

/// Округление требуемой мажорной версии до доступной LTS-сборки.
pub fn round_to_lts(major: u32) -> u32 {
    match major {
        0..=8 => 8,
        9..=17 => 17, // 9/11/16 -> 17 (обратно совместимо для игры)
        18..=21 => 21,
        _ => 25,
    }
}

/// Требуемая Java по id версии, когда в JSON нет `javaVersion`.
pub fn guess_major(id: &str, kind: &str) -> u32 {
    if kind == "old_beta" || kind == "old_alpha" {
        return 8;
    }
    let id_l = id.to_ascii_lowercase();
    // Снапшоты вида 24w14a / 21w19a
    if let Some((yy, rest)) = split_snapshot(&id_l) {
        let week: u32 = rest.parse().unwrap_or(0);
        return match yy {
            0..=20 => 8,
            21 => if week >= 19 { 17 } else { 8 },
            22 | 23 => 17,
            24 => if week >= 14 { 21 } else { 17 },
            25 => 21,
            _ => 25,
        };
    }
    // Релизы: 1.x[.y] или новая схема 26.x
    let nums: Vec<u32> = id_l
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .take(3)
        .filter_map(|s| s.parse().ok())
        .collect();
    match nums.as_slice() {
        [1, minor, rest @ ..] => {
            let patch = rest.first().copied().unwrap_or(0);
            match *minor {
                0..=16 => 8,
                17..=19 => 17,
                20 => if patch >= 5 { 21 } else { 17 },
                21..=99 => 21,
                _ => 21,
            }
        }
        [major, ..] if *major >= 26 => 25,
        [major, ..] if *major >= 2 => 21,
        _ => 8,
    }
}

fn split_snapshot(id: &str) -> Option<(u32, String)> {
    let w = id.find('w')?;
    let (yy, rest) = id.split_at(w);
    if yy.len() != 2 || !yy.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let digits: String = rest[1..].chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    Some((yy.parse().ok()?, digits))
}

/// Итоговый выбор. `declared` — `javaVersion.majorVersion` из JSON (если есть).
pub fn required_java(declared: Option<u32>, id: &str, kind: &str) -> u32 {
    match declared {
        Some(m) if m > 0 => round_to_lts(m),
        _ => round_to_lts(guess_major(id, kind)),
    }
}

/// Для старых Forge (<=1.16.5) Java новее 8 ломает загрузку — фиксируем 8.
/// Для NeoForge/Forge 1.17+ берём то, что просит версия.
pub fn adjust_for_loader(base: u32, loader: &str, mc: &str) -> u32 {
    if loader == "forge" && guess_major(mc, "release") == 8 {
        return 8;
    }
    base
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declared_is_rounded() {
        assert_eq!(required_java(Some(8), "1.12.2", "release"), 8);
        assert_eq!(required_java(Some(16), "1.17.1", "release"), 17);
        assert_eq!(required_java(Some(17), "1.18.2", "release"), 17);
        assert_eq!(required_java(Some(21), "1.21", "release"), 21);
        assert_eq!(required_java(Some(25), "26.1", "release"), 25);
        assert_eq!(required_java(Some(11), "x", "release"), 17);
    }

    #[test]
    fn releases() {
        for (id, j) in [("1.7.10", 8), ("1.12.2", 8), ("1.16.5", 8), ("1.17.1", 17), ("1.19.4", 17),
                        ("1.20.1", 17), ("1.20.4", 17), ("1.20.5", 21), ("1.20.6", 21), ("1.21.4", 21), ("26.1", 25)] {
            assert_eq!(required_java(None, id, "release"), j, "{id}");
        }
    }

    #[test]
    fn old_beta_alpha_are_java8() {
        // Баг старого лаунчера: эти версии получали Java 21.
        assert_eq!(required_java(None, "b1.7.3", "old_beta"), 8);
        assert_eq!(required_java(None, "a1.2.6", "old_alpha"), 8);
        assert_eq!(required_java(None, "rd-132211", "old_alpha"), 8);
    }

    #[test]
    fn snapshots() {
        assert_eq!(required_java(None, "13w47a", "snapshot"), 8);
        assert_eq!(required_java(None, "21w10a", "snapshot"), 8);
        assert_eq!(required_java(None, "21w19a", "snapshot"), 17);
        assert_eq!(required_java(None, "23w31a", "snapshot"), 17);
        assert_eq!(required_java(None, "24w13a", "snapshot"), 17);
        assert_eq!(required_java(None, "24w14a", "snapshot"), 21);
    }

    #[test]
    fn old_forge_stays_on_8() {
        assert_eq!(adjust_for_loader(17, "forge", "1.16.5"), 8);
        assert_eq!(adjust_for_loader(17, "forge", "1.18.2"), 17);
        assert_eq!(adjust_for_loader(21, "neoforge", "1.21.1"), 21);
    }
}

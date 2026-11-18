# Redshift Launcher (Rust)

Нативный лаунчер без Electron/браузерного движка: Rust + egui. Интерфейс перерисовывается только по событиям.

## Сборка (Windows)
1. Установите Rust: https://rustup.rs (MSVC toolchain).
2. `build.bat` (или `cargo build --release -p redshift-launcher`) → `target\release\redshift-launcher.exe`.
3. Без установки Rust: запушьте в GitHub — Actions («Windows build») соберёт `.exe` и portable-zip.
Portable: положите папку `data` рядом с exe — все данные будут в ней.

## Тесты
`cargo test -p redshift-core` (62 теста: запуск, Java, JVM, загрузчики, Modrinth, настройки, аккаунты).

## Что исправлено для «не запускаются Forge / сборки / старые версии»
- Java выбирается по `javaVersion` из JSON Mojang → LTS 8/17/21/25; для old_beta/old_alpha/снапшотов — эвристика (раньше им давалась Java 21). Forge ≤1.16.5 всегда Java 8.
- Библиотеки загрузчика перекрывают ванильные (ASM и др.), client.jar берётся у базовой версии.
- RAM ограничивается объёмом ПК; на слабых ПК автоматически пресет lowend.
- При «Unrecognized VM option» / «Could not reserve enough space» запуск повторяется с безопасными флагами / меньшим RAM.

Не перенесено и ограничения — см. MIGRATION_HANDOFF.md.

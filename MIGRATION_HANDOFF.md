# MIGRATION_HANDOFF — миграция Redshift Launcher Electron → Rust

Состояние: **частичная миграция (≈50–60% функций), НЕ 100%.** Старый проект: `/home/claude/BaseLauncher` (Electron).

## Сделано (проверено тестами на Linux, 62 теста)
`core/src/`: `java.rs` (выбор Java), `jvm.rs` (пресеты, лимит RAM), `diag.rs` (разбор сбоев), `launch.rs` (замена MCLC: правила, inheritsFrom-слияние, classpath, natives, старый/новый формат аргументов), `net.rs` (загрузки: пул потоков, sha1, атомарная запись, повторы), `install.rs`, `javart.rs` (Adoptium JRE), `runner.rs`, `loaders.rs` (Fabric/Quilt/Forge/NeoForge), `modrinth.rs` (поиск, установка модов, импорт/экспорт .mrpack), `settings.rs`, `accounts.rs` (offline, Ely.by), `play.rs` (оркестрация + автоповтор).
`app/src/main.rs`: GUI egui — Играть, Сборки, Моды (Modrinth), Аккаунты (offline/Ely.by), Настройки.

## НЕ перенесено (в Electron-версии есть)
- Microsoft-аккаунты (device code → Xbox → XSTS → Minecraft); логика в `BaseLauncher/src/msauth.js`, тесты `test/msauth.test.js`.
- REDSHIFT ID (Supabase): `src/supabase.js`, `src/config.js` (ключ в старом config.js мог быть утерян — вводить заново).
- 3D-просмотр скина, загрузка скина, локальный skin-сервер (`skinserver.js`).
- CurseForge и импорт CurseForge-сборок (`src/modpacks.js`), каталог сборок.
- Оптимизация игры (`perf.js`: options.txt, FPS-моды), темы/акценты, обновление.
- Импорт .mrpack есть в ядре (`modrinth::import_mrpack`), но кнопки в GUI нет; экспорт — кнопка в Настройках.
- Отмена загрузки/запуска, прогресс в байтах.
- Старые данные Electron (settings.json) не импортируются — формат новый `redshift.json`.
- Шифрование токена Ely.by (сейчас в открытом JSON).
- Повторная установка Forge, если версия уже есть, но `find_existing` ошибся по имени — проверить на реальных сборках.

## Не проверено
- Windows-сборка, реальные Mojang/Modrinth/Ely/Adoptium/Forge (в песочнице сеть к ним закрыта). Только mock-серверы и заглушки.
- Реальный запуск Minecraft ни разу не выполнялся.

## Замеры (Linux, Xvfb + Mesa llvmpipe, release)
Простой CPU 0.00% за 10 с; RSS 116 МБ (с программным OpenGL; на Windows с GPU-драйвером будет иначе); exe 6.1 МБ (Linux). Старый Electron-вариант не измерялся — запустить его в песочнице нельзя (npm закрыт). Время запуска не измерено.

## Дальше
1. Запустить GitHub Action, взять exe, проверить реальный запуск 1.20.1 vanilla/Fabric/Forge, 1.7.10, b1.7.3.
2. Перенести msauth → `core/src/msauth.rs`, supabase → `core/src/supabase.rs` (тесты из `test/*.test.js` как спецификация).
3. CurseForge, скины (egui_glow мешь для 3D), perf.rs.
4. Кнопки импорта .mrpack, отмена, импорт старых настроек.

## Обновление (ассеты и дизайн)
- Красный Electron-проект восстановлен из журнала сессии; его исходники и перегенерированные ассеты лежат в `legacy-ui/` и `assets/` (см. ASSET_MIGRATION_REPORT.md). Прикреплённый BaseLauncher.zip — это ПЕРВАЯ зелёная версия, не Redshift.
- Новое в Rust: `app/src/theme.rs` (палитра, фон, шрифт, ассеты через include_bytes!), `app/src/widgets.rs` (стеклянные карточки, градиентные кнопки, иконки), `core/src/perf.rs` («Оптимизировать FPS»: options.txt + FPS-моды), `app/build.rs` (иконка exe через winresource — не проверено на Windows).
- Страница «Скины»: только плоский предпросмотр. Следующие шаги по дизайну: 3D-скин (порт `legacy-ui/skin3d.js`), вкладки настроек, темы акцентов (`legacy-ui/app.js` ACCENTS), жирный шрифт (подключить Segoe UI Bold отдельным семейством).

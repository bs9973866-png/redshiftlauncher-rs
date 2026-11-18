@echo off
cargo test -p redshift-core --release && cargo build --release -p redshift-launcher
echo EXE: target\release\redshift-launcher.exe
pause

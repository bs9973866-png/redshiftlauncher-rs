// Иконка .exe (только при сборке под Windows). Файл берётся из assets/, поэтому не зависит от старого проекта.
fn main() {
    println!("cargo:rerun-if-changed=../assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../assets/icon.ico");
        res.set("ProductName", "Redshift Launcher");
        res.set("FileDescription", "Redshift Launcher");
        if let Err(e) = res.compile() { println!("cargo:warning=не удалось встроить иконку: {e}"); }
    }
}

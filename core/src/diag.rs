//! Разбор лога/кода выхода игры -> понятная причина и действие.

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Fix {
    /// Повторить с минимальными JVM-флагами.
    RetrySafeJvm,
    /// Повторить с меньшим объёмом RAM.
    LowerRam,
    /// Нужна другая (более старая/новая) Java.
    WrongJava,
    /// Нужно больше RAM.
    MoreRam,
    None,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Diagnosis {
    pub fix: Fix,
    pub message: &'static str,
}

pub fn analyze(log: &str) -> Diagnosis {
    let l = log;
    let has = |s: &str| l.contains(s);
    if has("Unrecognized VM option") || has("Unrecognized option") || has("Invalid argument") && has("XX:") {
        return Diagnosis { fix: Fix::RetrySafeJvm, message: "Эта версия Java не знает один из JVM-флагов. Перезапуск с безопасными флагами." };
    }
    if has("Could not reserve enough space") || has("Failed to commit") || has("Cannot allocate memory") || has("There is insufficient memory") {
        return Diagnosis { fix: Fix::LowerRam, message: "Не хватает свободной памяти под выбранный объём RAM. Уменьшаю RAM." };
    }
    if has("UnsupportedClassVersionError") || has("Unsupported class file major version") || has("class file version 6") {
        return Diagnosis { fix: Fix::WrongJava, message: "Версия Java не подходит этой версии игры/загрузчика." };
    }
    if has("java.lang.OutOfMemoryError") || has("Java heap space") || has("GC overhead limit exceeded") {
        return Diagnosis { fix: Fix::MoreRam, message: "Игре не хватило памяти. Увеличьте RAM для этой сборки." };
    }
    if has("Error: Could not create the Java Virtual Machine") || has("A fatal exception has occurred") {
        return Diagnosis { fix: Fix::RetrySafeJvm, message: "JVM не запустилась. Повтор с безопасными флагами." };
    }
    Diagnosis { fix: Fix::None, message: "" }
}

/// Решение лаунчера о повторе: не больше одного автоповтора каждого вида.
pub fn should_retry(d: &Diagnosis, already_safe: bool, already_lowered: bool) -> bool {
    match d.fix {
        Fix::RetrySafeJvm => !already_safe,
        Fix::LowerRam => !already_lowered,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_unknown_flag() {
        let d = analyze("Unrecognized VM option 'G1NewSizePercent'\nError: Could not create the Java Virtual Machine.");
        assert_eq!(d.fix, Fix::RetrySafeJvm);
    }
    #[test]
    fn detects_memory() {
        assert_eq!(analyze("Error occurred during initialization of VM\nCould not reserve enough space for 4194304KB object heap").fix, Fix::LowerRam);
    }
    #[test]
    fn detects_wrong_java() {
        assert_eq!(analyze("java.lang.UnsupportedClassVersionError: net/minecraft/client/main/Main has been compiled by a more recent version").fix, Fix::WrongJava);
        assert_eq!(analyze("Unsupported class file major version 65").fix, Fix::WrongJava);
    }
    #[test]
    fn detects_oom() {
        assert_eq!(analyze("Exception in thread \"main\" java.lang.OutOfMemoryError: Java heap space").fix, Fix::MoreRam);
    }
    #[test]
    fn normal_log_is_none() {
        assert_eq!(analyze("[Render thread/INFO]: Stopping!").fix, Fix::None);
    }
    #[test]
    fn retry_only_once() {
        let d = analyze("Unrecognized VM option 'X'");
        assert!(should_retry(&d, false, false));
        assert!(!should_retry(&d, true, false));
    }
}

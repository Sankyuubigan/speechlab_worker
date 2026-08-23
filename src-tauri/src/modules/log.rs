use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

use tauri::{AppHandle, Emitter};

/// Вычисляет путь к файлу `test/last_logs`.
///
/// Поднимаемся от `current_exe().parent()` вверх по каталогам, пока не найдём
/// папку, содержащую `test/`. Если не найдено — создаём `test/` рядом с exe.
/// Хардкод абсолютных путей запрещён (global core rules §1.4).
pub fn last_logs_path() -> PathBuf {
    let start = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    let mut dir = start;
    loop {
        if dir.join("test").is_dir() {
            return dir.join("test").join("last_logs");
        }
        match dir.parent() {
            Some(parent) => dir = parent.to_path_buf(),
            None => break,
        }
    }

    let fallback = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("test");
    let _ = std::fs::create_dir_all(&fallback);
    fallback.join("last_logs")
}

/// Очищает (truncate) файл last_logs при старте сессии.
pub fn truncate_last_logs() {
    let path = last_logs_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, "");
}

/// Единая точка записи логов (требование global core rules §2.5.1):
/// stderr + UI-событие `app-log` + файл `test/last_logs`.
pub fn app_log(app: &AppHandle, msg: &str) {
    eprintln!("{msg}");
    let _ = app.emit("app-log", msg);
    if let Err(e) = write_to_file(msg) {
        eprintln!("[logger] не удалось записать last_logs: {e}");
    }
}

fn write_to_file(msg: &str) -> std::io::Result<()> {
    let path = last_logs_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
    writeln!(file, "{msg}")?;
    Ok(())
}

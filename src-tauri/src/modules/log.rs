use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use log::{LevelFilter, Metadata, Record};
use tauri::{AppHandle, Emitter};

/// Сериализует запись в `last_logs`: в файл пишут несколько потоков одновременно
/// (stderr-поток движка, основной поток ошибок, таймеры), и без блокировки их записи
/// перемешивались, терялся `\n` и строки склеивались (файл выглядел
/// «не текстовым»). Мьютекс гарантирует атомарность каждой строки.
static LOG_MUTEX: Mutex<()> = Mutex::new(());

/// AppHandle, сохранённый при установке логгера (нужен для `app-log` emit).
static APP_HANDLE: OnceLock<AppHandle> = OnceLock::new();

/// Единый кастомный логгер (core rules §2.5): все уровни `log::` идут в
/// stderr + UI-событие `app-log` + файл `test/last_logs`. Одна точка записи,
/// единый формат таймстемпов (§2.5.1), без дублирующих emit вручную.
struct AppLogger;

impl log::Log for AppLogger {
    fn enabled(&self, _metadata: &Metadata) -> bool {
        true
    }

    fn log(&self, record: &Record) {
        let line = format!("[{}] {}", timestamp(), record.args());
        eprintln!("{line}");
        if let Some(app) = APP_HANDLE.get() {
            let _ = app.emit("app-log", line.clone());
        }
        if let Err(e) = write_to_file(&line) {
            eprintln!("[logger] не удалось записать last_logs: {e}");
        }
    }

    fn flush(&self) {}
}

/// Устанавливает глобальный логгер. Вызывается один раз в `setup()` приложения.
pub fn install(app: AppHandle) {
    let _ = APP_HANDLE.set(app);
    let _ = log::set_boxed_logger(Box::new(AppLogger));
    log::set_max_level(LevelFilter::Debug);
}

/// Вычисляет путь к файлу `test/last_logs`.
///
/// Поднимаемся от `current_exe().parent()` вверх по каталогам:
/// 1. первый предок, похожий на корень проекта (рядом есть `src-tauri/`) —
///    адрес `корень/test/last_logs.txt`; папка `test/` создаётся сама,
///    поэтому случайное удаление её не ломает запись логов в проект;
/// 2. иначе первый предок с существующей `test/` (не внутри `target/`);
/// 3. если ничего не найдено (упакованный бинарь) — создаём `test/` рядом с exe.
/// Хардкод абсолютных путей запрещён (global core rules §1.4).
pub fn last_logs_path() -> PathBuf {
    let start = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    let mut dir = start;
    loop {
        if dir.join("src-tauri").is_dir() {
            return dir.join("test").join("last_logs.txt");
        }
        if dir.join("test").is_dir() && !is_inside_target(&dir) {
            return dir.join("test").join("last_logs.txt");
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
    fallback.join("last_logs.txt")
}

/// `target/debug`, `target/release` и т.п. не являются корнем проекта: не даём
/// логировать в зомби-папку `target/.../test` (удаляется при каждой чистой сборке).
fn is_inside_target(dir: &PathBuf) -> bool {
    let name = dir
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    name == "target"
        || name == "debug"
        || name == "release"
        || dir
            .ancestors()
            .any(|a| a.file_name().map(|s| s == "target").unwrap_or(false))
}

/// Очищает (truncate) файл last_logs при старте сессии.
pub fn truncate_last_logs() {
    let path = last_logs_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, "");
}

/// Метка времени события для каждой строки лога.
///
/// Формат `ГГГГ-ММ-ДД ЧЧ:ММ:СС` (локальное время через `chrono`, если доступно;
/// см. Cargo.toml). Единая точка простановки времени — гарантирует идентичный
/// тайминг в файле `last_logs` и во вкладке «Логи» UI (SSOT, core rules §2.5.1).
fn timestamp() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// Совместимый вызов для старого кода: маршрутизирует через глобальный логгер
/// (тот же формат, что и `log::` макросы). Не дублирует запись вручную.
pub fn app_log(_app: &AppHandle, msg: &str) {
    log::info!("{msg}");
}

fn write_to_file(msg: &str) -> std::io::Result<()> {
    let _guard = LOG_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let path = last_logs_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
    writeln!(file, "{msg}")?;
    Ok(())
}
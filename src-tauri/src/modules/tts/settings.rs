use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::Manager;

/// Сохраняемые настройки TTS-движка CrispASR.
///
/// Единственный источник правды о путях к движку/моделям и выбранном пресете.
/// Сериализуется в `tts_settings.json` внутри app-config dir (100% локально, без облака).
///
/// Пути к конкретным GGUF-файлам модели/кодека/голоса НЕ храним — они всегда
/// восстанавливаются из `models_dir/<preset_id>/<файл>` при запуске движка.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TtsSettings {
    /// Папка, где живут бинари движка по бэкендам: `<engine_dir>/<backend>/crispasr.exe`.
    pub engine_dir: String,
    /// ОБЩАЯ папка моделей TTS. Внутри — подпапка на каждый пресет.
    pub models_dir: String,
    /// Выбранный тип бэкенда движка: "cpu" / "cuda" / "vulkan" / "cpu-legacy" / …
    pub engine_backend: String,
    /// Выбранный пресет TTS-модели.
    pub preset: String,
}

const FILE_NAME: &str = "tts_settings.json";

fn config_path(app: &tauri::AppHandle) -> PathBuf {
    app.path()
        .app_config_dir()
        .unwrap_or_else(|_| {
            std::env::current_exe()
        .unwrap_or_else(|_| PathBuf::from("."))
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .to_path_buf()
        })
        .join(FILE_NAME)
}

/// Загружает настройки из диска. При отсутствии/невалидном файле возвращает дефолтные
/// (все поля пусты — UI сам подставит разумные значения, например пресет по умолчанию).
pub fn load(app: &tauri::AppHandle) -> TtsSettings {
    let path = config_path(app);
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
        Err(_) => TtsSettings::default(),
    }
}

/// Сохраняет настройки на диск.
pub fn save(app: &tauri::AppHandle, settings: &TtsSettings) -> Result<(), String> {
    let path = config_path(app);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let text = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| format!("не удалось сохранить настройки: {e}"))
}

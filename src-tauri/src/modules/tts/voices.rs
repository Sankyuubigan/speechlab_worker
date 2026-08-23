use std::path::PathBuf;

use serde::Serialize;
use tauri::Emitter;

use crate::modules::audio::wav;

/// Информация о сохранённом голосе из хранилища `<models_dir>/voices/<id>/`.
#[derive(Debug, Clone, Serialize)]
pub struct VoiceInfo {
    /// Уникальный id (санированное имя папки).
    pub id: String,
    /// Отображаемое имя (то, что дал пользователь).
    pub name: String,
    /// Абсолютный путь к `ref_audio.wav` (для передачи в `--voice`/тело запроса).
    pub path: String,
    /// Референсный текст (если указан).
    pub ref_text: String,
    /// Есть ли `avatar.jpg` у голоса.
    pub has_avatar: bool,
    /// ISO-время создания (RFC3339).
    pub created_at: String,
}

/// Корень хранилища голосов: `<models_dir>/voices`.
pub fn voices_root(models_dir: &str) -> PathBuf {
    let base: PathBuf = if models_dir.is_empty() {
        crate::modules::tts::download::default_models_dir()
    } else {
        PathBuf::from(models_dir)
    };
    base.join("voices")
}

/// Санирует имя пользователя в безопасный id папки:
/// не-алфавитно-цифровые символы → `_`, схлопывание повторов, обрезка краёв.
fn sanitize_id(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut last_underscore = true; // не начинаем с _
    for ch in name.chars() {
        if ch.is_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_underscore = false;
        } else if !last_underscore {
            out.push('_');
            last_underscore = true;
        }
    }
    let trimmed = out.trim_matches('_').to_string();
    if trimmed.is_empty() {
        "voice".to_string()
    } else {
        trimmed
    }
}

/// Возвращает манифест голоса (`voice.json`) либо собирает минимальный из наличия файлов.
fn read_voice(root: &std::path::Path, id: &str) -> Option<VoiceInfo> {
    let folder = root.join(id);
    let wav = folder.join("ref_audio.wav");
    if !wav.exists() {
        return None;
    }
    let manifest = folder.join("voice.json");
    let (name, ref_text, has_avatar, created_at) = if let Ok(text) =
        std::fs::read_to_string(&manifest)
    {
        let v: serde_json::Value = serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
        (
            v["name"].as_str().unwrap_or(id).to_string(),
            v["ref_text"].as_str().unwrap_or("").to_string(),
            v["has_avatar"].as_bool().unwrap_or(false),
            v["created_at"].as_str().unwrap_or("").to_string(),
        )
    } else {
        (id.to_string(), String::new(), folder.join("avatar.jpg").exists(), String::new())
    };
    Some(VoiceInfo {
        id: id.to_string(),
        name,
        path: wav.to_string_lossy().to_string(),
        ref_text,
        has_avatar,
        created_at,
    })
}

/// Сканирует хранилище и возвращает список сохранённых голосов.
pub fn list_voices(models_dir: &str) -> Vec<VoiceInfo> {
    let root = voices_root(models_dir);
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&root) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                    if let Some(info) = read_voice(&root, name) {
                        out.push(info);
                    }
                }
            }
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Добавляет голос в хранилище.
///
/// `src_audio` декодируется (любой формат: ogg/wav/flac/mp3/opus) и конвертируется в
/// моно-WAV с **оригинальной** частотой дискретизации (`decode_to_mono` + `wav::write_wav`).
/// Рядом сохраняются `ref_text.txt`, опционально `avatar.jpg` и манифест `voice.json`.
///
/// Если голос с таким именем уже есть — к id добавляется суффикс `_2`, `_3`, …
pub fn add_voice(
    app: &tauri::AppHandle,
    models_dir: &str,
    name: &str,
    src_audio: &str,
    ref_text: &str,
    avatar: &str,
) -> Result<VoiceInfo, String> {
    if name.trim().is_empty() {
        return Err("укажите имя голоса".into());
    }
    if !std::path::Path::new(src_audio).exists() {
        return Err(format!("файл аудио не найден: {src_audio}"));
    }

    let root = voices_root(models_dir);
    std::fs::create_dir_all(&root)
        .map_err(|e| format!("не удалось создать папку голосов {}: {e}", root.display()))?;

    // Резолвим уникальный id (с суффиксом при коллизии).
    let base_id = sanitize_id(name);
    let mut id = base_id.clone();
    let mut suffix = 2u32;
    while root.join(&id).exists() {
        id = format!("{base_id}_{suffix}");
        suffix += 1;
    }
    let folder = root.join(&id);
    std::fs::create_dir_all(&folder)
        .map_err(|e| format!("не удалось создать папку голоса {}: {e}", folder.display()))?;

    // Конвертация аудио -> моно WAV (ориг. частота).
    let _ = app.emit("app-log", format!("[voices] конвертирую «{name}» в WAV..."));
    let (mono, rate) = crate::modules::audio::decode_to_mono(src_audio)
        .map_err(|e| format!("не удалось декодировать аудио «{src_audio}»: {e}"))?;
    if mono.is_empty() {
        return Err("декодированное аудио пустое (тишина?)".into());
    }
    let wav_path = folder.join("ref_audio.wav");
    wav::write_wav(&wav_path.to_string_lossy(), &mono, rate)
        .map_err(|e| format!("не удалось записать WAV: {e}"))?;

    // Референсный текст.
    let _ = std::fs::write(folder.join("ref_text.txt"), ref_text.trim());

    // Аватар (опционально) — просто копируем, если это похоже на картинку.
    let mut has_avatar = false;
    if !avatar.is_empty() && std::path::Path::new(avatar).exists() {
        let ext = std::path::Path::new(avatar)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "webp") {
            let _ = std::fs::copy(avatar, folder.join("avatar.jpg"));
            has_avatar = true;
        }
    }

    let created_at = chrono_now();
    let manifest = serde_json::json!({
        "id": id,
        "name": name,
        "ref_text": ref_text.trim(),
        "has_avatar": has_avatar,
        "created_at": created_at,
    });
    let _ = std::fs::write(
        folder.join("voice.json"),
        serde_json::to_string_pretty(&manifest).unwrap_or_default(),
    );

    let _ = app.emit("app-log", format!("[voices] голос «{name}» сохранён ({})", wav_path.display()));
    Ok(VoiceInfo {
        id,
        name: name.to_string(),
        path: wav_path.to_string_lossy().to_string(),
        ref_text: ref_text.trim().to_string(),
        has_avatar,
        created_at,
    })
}

/// Удаляет голос (папку целиком) из хранилища.
pub fn delete_voice(models_dir: &str, id: &str) -> Result<(), String> {
    let folder = voices_root(models_dir).join(id);
    if !folder.exists() {
        return Err(format!("голос не найден: {id}"));
    }
    std::fs::remove_dir_all(&folder)
        .map_err(|e| format!("не удалось удалить голос {id}: {e}"))
}

/// RFC3339-подобное UTC-время (без внешних зависимостей — через std::time).
fn chrono_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    const MONTH_DAYS: [u32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let days = secs / 86400;
    let mut y = 1970u32;
    let mut rem = days;
    loop {
        let leap = if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 {
            366
        } else {
            365
        };
        if rem < leap {
            break;
        }
        rem -= leap;
        y += 1;
    }
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let mut m = 0usize;
    let mut d = rem;
    loop {
        let md = (MONTH_DAYS[m] + if m == 1 && leap { 1 } else { 0 }) as u64;
        if d < md {
            break;
        }
        d -= md;
        m += 1;
    }
    let day = d + 1;
    let month = m + 1;
    let hour = (secs % 86400) / 3600;
    let min = (secs % 3600) / 60;
    let sec = secs % 60;
    format!("{y}-{month:02}-{day:02}T{hour:02}:{min:02}:{sec:02}Z")
}

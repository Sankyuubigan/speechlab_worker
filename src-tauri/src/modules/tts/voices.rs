use std::path::PathBuf;

use serde::Serialize;

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

/// Возвращает манифест голоса (`<id>.json`) либо собирает минимальный из наличия файлов.
///
/// Хранилище — плоское: `<voice-dir>/<id>.wav` (+ `<id>.txt`, `<id>.json`,
/// `<id>.avatar.jpg`). Это требование CrispASR HTTP-сервера: поле `voice` в
/// `POST /v1/audio/speech` резолвится только против `--voice-dir`, и имя не
/// должно содержать разделителей путей (см. bug `400 invalid_voice`).
fn read_voice(root: &std::path::Path, id: &str) -> Option<VoiceInfo> {
    let wav = root.join(format!("{id}.wav"));
    if !wav.exists() {
        return None;
    }
    let manifest = root.join(format!("{id}.json"));
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
        (
            id.to_string(),
            String::new(),
            root.join(format!("{id}.avatar.jpg")).exists(),
            String::new(),
        )
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

/// Однократно переносит старые голоса из подпапок `<id>/ref_audio.wav` в плоскую
/// раскладку `<id>.wav` (требование сервера). Безопасно при отсутствии старых данных.
fn migrate_legacy(root: &std::path::Path) {
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                let ref_wav = p.join("ref_audio.wav");
                if ref_wav.exists() {
                    if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                        let _ = std::fs::copy(&ref_wav, root.join(format!("{name}.wav")));
                        let _ = std::fs::copy(
                            p.join("ref_text.txt"),
                            root.join(format!("{name}.txt")),
                        );
                        let _ = std::fs::copy(
                            p.join("avatar.jpg"),
                            root.join(format!("{name}.avatar.jpg")),
                        );
                        let _ = std::fs::copy(
                            p.join("voice.json"),
                            root.join(format!("{name}.json")),
                        );
                        let _ = std::fs::remove_dir_all(&p);
                    }
                }
            }
        }
    }
}

/// Сканирует хранилище и возвращает список сохранённых голосов.
///
/// В список НЕ попадают служебные/временные файлы:
/// - файлы, начинающиеся с `.` (скрытые);
/// - кэш-файлы клонирования `<id>.__clone.wav` (пишутся `prepare_clone_reference`).
/// Это предотвращает появление «истории загруженных аудио» в выпадающем списке
/// выбора клона голоса — там должны быть только явно сохранённые голоса.
pub fn list_voices(models_dir: &str) -> Vec<VoiceInfo> {
    let root = voices_root(models_dir);
    migrate_legacy(&root);
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&root) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_file() {
                let fname = entry.file_name();
                let fname_str = fname.to_string_lossy();
                // Пропускаем скрытые и кэш-файлы клонирования.
                if fname_str.starts_with('.') || fname_str.ends_with(".__clone.wav") {
                    continue;
                }
                if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
                    if ext.eq_ignore_ascii_case("wav") {
                        if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                            // Повторная проверка на кэш по стему (на случай странных имён).
                            if stem.ends_with(".__clone") {
                                continue;
                            }
                            if let Some(info) = read_voice(&root, stem) {
                                out.push(info);
                            }
                        }
                    }
                }
            }
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Резолвит значение поля `voice` для HTTP-запроса к CrispASR.
///
/// Возвращает значение как есть (bare-id или путь к файлу). **ВАЖНО:** функция
/// больше НЕ копирует WAV-файлы в хранилище голосов — иначе любой выбранный
/// «свой WAV без сохранения» попадал бы в выпадающий список клонов как
/// сохранённый голос (баг с «историей загруженных аудио»). Клонирование из
/// произвольного пути обрабатывается в `tts_speak` через `prepare_clone_reference`,
/// которому передаётся исходный путь напрямую (сервер cosyvoice3 принимает
/// `--voice <абсолютный путь>` к любому файлу).
pub fn resolve_voice_for_server(models_dir: &str, voice: &str) -> Result<String, String> {
    let v = voice.trim();
    if v.is_empty() {
        return Ok(String::new());
    }
    // Для пути к файлу просто проверяем существование (без копирования в стор).
    let looks_like_path = v.contains('/')
        || v.contains('\\')
        || std::path::Path::new(v).is_absolute()
        || v.to_ascii_lowercase().ends_with(".wav")
        || v.to_ascii_lowercase().ends_with(".gguf");
    if looks_like_path && !std::path::Path::new(v).exists() {
        return Err(format!("файл голоса не найден: {v}"));
    }
    let _ = models_dir;
    Ok(v.to_string())
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

    // Резолвим уникальный id (с суффиксом при коллизии). Голоса теперь плоские:
    // коллизия проверяется по `<id>.wav`.
    let base_id = sanitize_id(name);
    let mut id = base_id.clone();
    let mut suffix = 2u32;
    while root.join(format!("{id}.wav")).exists() {
        id = format!("{base_id}_{suffix}");
        suffix += 1;
    }

    // Конвертация аудио -> моно WAV (ориг. частота). Плоская раскладка:
    // `<voice-dir>/<id>.wav` — так его находит CrispASR-сервер по `--voice-dir`.
    crate::modules::log::app_log(app, &format!("[voices] конвертирую «{name}» в WAV..."));
    let (mono, rate) = crate::modules::audio::decode_to_mono(src_audio)
        .map_err(|e| format!("не удалось декодировать аудио «{src_audio}»: {e}"))?;
    if mono.is_empty() {
        return Err("декодированное аудио пустое (тишина?)".into());
    }
    let wav_path = root.join(format!("{id}.wav"));
    wav::write_wav(&wav_path.to_string_lossy(), &mono, rate)
        .map_err(|e| format!("не удалось записать WAV: {e}"))?;

    // Референсный текст (сервер автоматически подхватит `<id>.txt` как ref_text).
    let _ = std::fs::write(root.join(format!("{id}.txt")), ref_text.trim());

    // Аватар (опционально) — просто копируем, если это похоже на картинку.
    let mut has_avatar = false;
    if !avatar.is_empty() && std::path::Path::new(avatar).exists() {
        let ext = std::path::Path::new(avatar)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "webp") {
            let _ = std::fs::copy(avatar, root.join(format!("{id}.avatar.jpg")));
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
        root.join(format!("{id}.json")),
        serde_json::to_string_pretty(&manifest).unwrap_or_default(),
    );

    crate::modules::log::app_log(app, &format!("[voices] голос «{name}» сохранён ({})", wav_path.display()));
    Ok(VoiceInfo {
        id,
        name: name.to_string(),
        path: wav_path.to_string_lossy().to_string(),
        ref_text: ref_text.trim().to_string(),
        has_avatar,
        created_at,
    })
}

/// Удаляет голос (плоские файлы `<id>.*` и, для совместимости, старую папку) из хранилища.
pub fn delete_voice(models_dir: &str, id: &str) -> Result<(), String> {
    let root = voices_root(models_dir);
    let wav = root.join(format!("{id}.wav"));
    let txt = root.join(format!("{id}.txt"));
    let manifest = root.join(format!("{id}.json"));
    let avatar = root.join(format!("{id}.avatar.jpg"));
    let legacy_folder = root.join(id);
    let mut removed = false;
    for f in [&wav, &txt, &manifest, &avatar] {
        if f.exists() {
            let _ = std::fs::remove_file(f);
            removed = true;
        }
    }
    if legacy_folder.exists() {
        let _ = std::fs::remove_dir_all(&legacy_folder);
        removed = true;
    }
    if !removed {
        return Err(format!("голос не найден: {id}"));
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmp_models_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("speechlab_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn sanitize_id_basic() {
        assert_eq!(sanitize_id("Morgan Freeman"), "morgan_freeman");
        assert_eq!(sanitize_id("голос-1!"), "1");
        assert_eq!(sanitize_id("  "), "voice");
        assert_eq!(sanitize_id("Влад"), "1".to_string()); // кириллица -> "voice"? проверяем что не пусто
        assert!(!sanitize_id("Влад").is_empty());
    }

    #[test]
    fn resolve_does_not_pollute_store() {
        let models = tmp_models_dir();
        // Внешний WAV вне хранилища голосов.
        let outside = std::env::temp_dir().join(format!("outside_{}.wav", std::process::id()));
        {
            let mut f = std::fs::File::create(&outside).unwrap();
            f.write_all(b"RIFF....WAVE").unwrap();
        }
        let res = resolve_voice_for_server(&models.to_string_lossy(), &outside.to_string_lossy());
        assert!(res.is_ok(), "resolve не должен падать на существующем файле");
        // Никакой копии в хранилище появиться не должно.
        let vr = voices_root(&models.to_string_lossy());
        let entries: Vec<_> = std::fs::read_dir(&vr).map(|r| r.flatten().collect()).unwrap_or_default();
        assert!(entries.is_empty(), "resolve_voice_for_server не должен копировать файлы в хранилище");
        let _ = std::fs::remove_file(&outside);
    }

    #[test]
    fn list_voices_excludes_clone_cache_and_hidden() {
        let models = tmp_models_dir();
        let root = voices_root(&models.to_string_lossy());
        std::fs::create_dir_all(&root).unwrap();
        // Сохранённый голос (должен попасть).
        std::fs::write(root.join("my_voice.wav"), b"RIFF....WAVE").unwrap();
        std::fs::write(root.join("my_voice.json"), r#"{"name":"My Voice"}"#).unwrap();
        // Кэш клонирования (НЕ должен попасть).
        std::fs::write(root.join("my_voice.__clone.wav"), b"RIFF....WAVE").unwrap();
        // Скрытый файл (НЕ должен попасть).
        std::fs::write(root.join(".hidden.wav"), b"RIFF....WAVE").unwrap();

        let voices = list_voices(&models.to_string_lossy());
        assert_eq!(voices.len(), 1, "в списке только сохранённый голос");
        assert_eq!(voices[0].id, "my_voice");
        assert_eq!(voices[0].name, "My Voice");

        let _ = std::fs::remove_dir_all(&models);
    }
}

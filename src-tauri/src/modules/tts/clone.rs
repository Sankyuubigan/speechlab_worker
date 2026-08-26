use crate::modules::tts::voices;

/// Лимиты длительности референса (в секундах) для моделей TTS, клонирующих голос
/// из произвольного WAV (zero-shot cloning). Возвращает `(min_seconds, max_seconds)`.
///
/// `None` — бэкенд не клонирует из WAV в рантайме через `--voice` (baked-имена
/// GGUF / `.npz` / `.json` / пресеты kokoro/orpheus и т.п., либо требует офлайн-
/// bake). Для таких используется старый путь регистрации `POST /v1/voices`.
///
/// Источник лимитов — `docs/tts.md` CrispASR:
/// - cosyvoice3: 4–10 с оптимально, 3–15 с допустимо; длинный референс портит
///   выхлоп (issue #334) → жёсткий cap 10 с.
/// - f5-tts / zonos / voxcpm2 / dots / irodori / indextts / omnivoice / moss /
///   confucius4 / pocket / vibevoice-1.5b / tada: 3–15 с → cap 15 с.
pub fn clone_reference_limits(backend: &str) -> Option<(f32, f32)> {
    match backend {
        "cosyvoice3-tts" | "cosyvoice3-tts-rl" => Some((3.0, 10.0)),
        "f5-tts" => Some((3.0, 15.0)),
        "zonos" => Some((3.0, 15.0)),
        "voxcpm2-tts" => Some((3.0, 15.0)),
        "dots-tts" => Some((3.0, 15.0)),
        "irodori-tts" => Some((3.0, 15.0)),
        "indextts" => Some((3.0, 15.0)),
        "omnivoice" => Some((3.0, 15.0)),
        "moss-tts" | "moss-tts-local" => Some((3.0, 15.0)),
        "confucius4-tts" => Some((3.0, 15.0)),
        "pocket-tts" => Some((3.0, 15.0)),
        "vibevoice-1.5b" => Some((3.0, 15.0)),
        "tada" | "tada-1b" | "tada-3b-ml" => Some((5.0, 15.0)),
        _ => None,
    }
}

/// Подготовленный референс для клонирования.
pub struct CloneRef {
    /// Абсолютный путь к (возможно обрезанному) WAV-референсу. Передаётся в
    /// `--voice <path>` при старте сервера — это надёжный путь клонирования для
    /// cosyvoice3/zonos/… (per-request `voice=<name>.wav` эти бэкенды игнорируют
    /// и ищут только baked-банку, отсюда `voice ... not found (have 8)`).
    pub voice_path: String,
    /// Референсный текст (транскрипт) из `<id>.txt`, если есть; иначе пусто
    /// (сервер авто-транскрибирует — безопаснее при неточном тексте, #334).
    pub ref_text: String,
}

/// Готовит WAV-референс для клонирования: при наличии лимита длительности
/// обрезает исходник до первых `max` секунд (кэш `<ascii_id>.__clone.wav`,
/// пересоздаётся при изменении исходника) и возвращает путь + ref_text.
///
/// `src_wav` — абсолютный путь к исходному референсному WAV (может лежать где
/// угодно, в т.ч. вне хранилища голосов — напр. «свой WAV без сохранения»).
/// `id` — стабильный идентификатор для имени кэш-файла; для защиты от багов
/// чтения не-ASCII путей сервером кэш всегда пишется под **ASCII**-именем
/// (`ascii_voice_name`), иначе cosyvoice3 не может открыть файл и падает 500
/// («could not read ... as PCM16 WAV»).
pub fn prepare_clone_reference(
    models_dir: &str,
    src_wav: &str,
    id: &str,
    backend: &str,
) -> Result<CloneRef, String> {
    let src = std::path::Path::new(src_wav);
    if !src.exists() {
        return Err(format!(
            "файл голоса не найден: {src_wav} (сначала добавьте голос или выберите WAV)"
        ));
    }

    let root = voices::voices_root(models_dir);
    std::fs::create_dir_all(&root)
        .map_err(|e| format!("не удалось создать папку голосов {}: {e}", root.display()))?;
    let cache_id = crate::modules::tts::ascii_voice_name(id);

    let (voice_path, ref_text) = match clone_reference_limits(backend) {
        Some((_min, max)) => {
            // Кэш клонирования — в нейтральной служебной папке (не внутри <id>/).
            let cache_dir = root.join(".clone_cache");
            let _ = std::fs::create_dir_all(&cache_dir);
            let trimmed = cache_dir.join(format!("{cache_id}.wav"));
            let need_rebuild = !trimmed.exists()
                || {
                    let src_m = std::fs::metadata(src).ok().and_then(|m| m.modified().ok());
                    let trm_m = std::fs::metadata(&trimmed).ok().and_then(|m| m.modified().ok());
                    match (src_m, trm_m) {
                        (Some(s), Some(t)) => s > t,
                        _ => true,
                    }
                };
            if need_rebuild {
                let (mono, rate) = crate::modules::audio::decode_to_mono(src_wav)
                    .map_err(|e| format!("не удалось декодировать референс «{id}»: {e}"))?;
                if mono.is_empty() {
                    return Err(format!("референс «{id}» пустой (тишина?)"));
                }
                let max_samples = (max * rate as f32) as usize;
                let taken = if mono.len() > max_samples {
                    &mono[..max_samples]
                } else {
                    &mono[..]
                };
                crate::modules::audio::wav::write_wav(&trimmed.to_string_lossy(), taken, rate)
                    .map_err(|e| format!("не удалось записать обрезанный референс: {e}"))?;
            }
            (trimmed.to_string_lossy().to_string(), read_ref_text(&root, id))
        }
        None => (src.to_string_lossy().to_string(), read_ref_text(&root, id)),
    };

    Ok(CloneRef { voice_path, ref_text })
}

fn read_ref_text(root: &std::path::Path, voice_id: &str) -> String {
    let txt = root.join(voice_id).join("ref_text.txt");
    if txt.exists() {
        std::fs::read_to_string(&txt).unwrap_or_default()
    } else {
        String::new()
    }
}

/// Возвращает байты **обрезанного** референса голоса (такой же, какой пойдёт в
/// клонирование, ≤ лимита длительности бэкенда). Используется для превью-проигрывания
/// в редакторе голоса. Бэкенд по умолчанию — `cosyvoice3-tts` (лимит 10 с).
pub fn voice_trimmed_audio(
    models_dir: &str,
    id: &str,
    backend: &str,
) -> Result<Vec<u8>, String> {
    let root = voices::voices_root(models_dir);
    let wav = root.join(id).join("voice.wav");
    if !wav.exists() {
        return Err(format!("файл голоса не найден: {id}"));
    }
    let backend = if backend.is_empty() {
        "cosyvoice3-tts"
    } else {
        backend
    };
    let cr = prepare_clone_reference(models_dir, &wav.to_string_lossy(), id, backend)?;
    std::fs::read(&cr.voice_path)
        .map_err(|e| format!("не удалось прочитать обрезанный референс {}: {e}", cr.voice_path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_known_backends() {
        assert_eq!(clone_reference_limits("cosyvoice3-tts"), Some((3.0, 10.0)));
        assert_eq!(clone_reference_limits("cosyvoice3-tts-rl"), Some((3.0, 10.0)));
        assert_eq!(clone_reference_limits("f5-tts"), Some((3.0, 15.0)));
        assert_eq!(clone_reference_limits("zonos"), Some((3.0, 15.0)));
        // Не клонирующие из WAV бэкенды — None.
        assert_eq!(clone_reference_limits("qwen3-tts"), None);
        assert_eq!(clone_reference_limits("kokoro"), None);
    }

    #[test]
    fn clone_cache_name_is_ascii() {
        // Имя кэш-файла клона (.__clone.wav) всегда должно быть ASCII, иначе
        // cosyvoice3 не открывает файл и падает 500.
        for id in ["Влад_без_текста", "Морган Фримен", "голос-1", "voice"] {
            let cache = format!("{}.__clone.wav", crate::modules::tts::ascii_voice_name(id));
            assert!(cache.chars().all(|c| c.is_ascii()), "не-ASCII в кэше: {cache}");
            assert!(cache.ends_with(".__clone.wav"));
        }
    }
}

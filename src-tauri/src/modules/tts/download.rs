use std::path::{Path, PathBuf};

use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::Emitter;
use tokio::io::AsyncWriteExt;

/// Адрес GitHub API последнего релиза CrispASR (для списка бинарей движка и проверки обновлений).
pub const RELEASE_API: &str = "https://api.github.com/repos/CrispStrobe/CrispASR/releases/latest";

/// Описание пресета TTS-движка CrispASR.
///
/// `backend` — имя бэкенда для `--backend` сервера. `model_url`/`codec`/`voice`/`extras` —
/// канонические GGUF из реестра CrispASR (HF `cstr/*`), скачиваются в одну папку;
/// сопутствующие файлы движок находит сам (sibling auto-discovery).
///
/// `voice_type` — как задаётся голос для этого движка:
/// - `"none"`    — голос не выбирается (фиксированный/один на модель)
/// - `"named"`   — по имени спикера (`--voice <name>`, встроенные имена в `builtin_voices`)
/// - `"clone"`   — клонирование из референсного WAV (`--voice ref.wav`)
/// - `"ggupack"` — GGUF voice-пак, скачивается вместе с моделью и грузится при старте
///
/// `supports_instruct` — передаёт ли движок стиль/описание голоса через `instructions`
/// (qwen3-tts-customvoice, parler-tts, irodori-tts, …).
#[derive(Clone, Copy)]
pub struct TtsPreset {
    pub id: &'static str,
    pub label: &'static str,
    pub backend: &'static str,
    pub model_file: &'static str,
    pub model_url: &'static str,
    pub codec: Option<(&'static str, &'static str)>,
    pub voice: Option<(&'static str, &'static str)>,
    pub extras: &'static [(&'static str, &'static str)],
    pub voice_type: &'static str,
    pub builtin_voices: &'static [&'static str],
    pub supports_instruct: bool,
}

pub const PRESETS: &[TtsPreset] = &[
    TtsPreset {
        id: "qwen3-tts",
        label: "Qwen3-TTS 0.6B (базовый, RU, голос-пак)",
        backend: "qwen3-tts",
        model_file: "qwen3-tts-12hz-0.6b-base-q8_0.gguf",
        model_url:
            "https://huggingface.co/cstr/qwen3-tts-0.6b-base-GGUF/resolve/main/qwen3-tts-12hz-0.6b-base-q8_0.gguf",
        codec: Some((
            "qwen3-tts-tokenizer-12hz.gguf",
            "https://huggingface.co/cstr/qwen3-tts-tokenizer-12hz-GGUF/resolve/main/qwen3-tts-tokenizer-12hz.gguf",
        )),
        voice: Some((
            "qwen3-tts-voice-default.gguf",
            "https://huggingface.co/cstr/qwen3-tts-voices-GGUF/resolve/main/qwen3-tts-voice-default.gguf",
        )),
        extras: &[],
        voice_type: "ggupack",
        builtin_voices: &[],
        supports_instruct: false,
    },
    TtsPreset {
        id: "qwen3-tts-customvoice",
        label: "Qwen3-TTS 1.7B CustomVoice (инструкции по стилю, RU)",
        backend: "qwen3-tts-customvoice",
        model_file: "qwen3-tts-12hz-1.7b-customvoice-q8_0.gguf",
        model_url:
            "https://huggingface.co/cstr/qwen3-tts-1.7b-customvoice-GGUF/resolve/main/qwen3-tts-12hz-1.7b-customvoice-q8_0.gguf",
        codec: Some((
            "qwen3-tts-tokenizer-12hz.gguf",
            "https://huggingface.co/cstr/qwen3-tts-tokenizer-12hz-GGUF/resolve/main/qwen3-tts-tokenizer-12hz.gguf",
        )),
        voice: None,
        extras: &[],
        voice_type: "named",
        builtin_voices: &["vivian", "ryan", "emma", "noah", "olivia", "liam", "sophia", "lucas", "mia"],
        supports_instruct: true,
    },
    TtsPreset {
        id: "parler-tts",
        label: "Parler-TTS Mini 1.1 (описание голоса словами, EN)",
        backend: "parler-tts",
        model_file: "parler-tts-mini-v1.1-q8_0.gguf",
        model_url:
            "https://huggingface.co/cstr/parler-tts-mini-v1.1-GGUF/resolve/main/parler-tts-mini-v1.1-q8_0.gguf",
        codec: None,
        voice: None,
        extras: &[],
        voice_type: "none",
        builtin_voices: &[],
        supports_instruct: true,
    },
    TtsPreset {
        id: "irodori-tts",
        label: "Irodori-TTS 500M v3 (клонирование WAV + эмоции, JA)",
        backend: "irodori-tts",
        model_file: "irodori-tts-500m-v3-q8_0.gguf",
        model_url:
            "https://huggingface.co/cstr/irodori-tts-GGUF/resolve/main/irodori-tts-500m-v3-q8_0.gguf",
        codec: Some((
            "dacvae-ja-32dim-f16.gguf",
            "https://huggingface.co/cstr/irodori-tts-GGUF/resolve/main/dacvae-ja-32dim-f16.gguf",
        )),
        voice: None,
        extras: &[],
        voice_type: "clone",
        builtin_voices: &[],
        supports_instruct: true,
    },
    TtsPreset {
        id: "kokoro",
        label: "Kokoro 82M (быстрый, мультиязычный)",
        backend: "kokoro",
        model_file: "kokoro-82m-q8_0.gguf",
        model_url: "https://huggingface.co/cstr/kokoro-82m-GGUF/resolve/main/kokoro-82m-q8_0.gguf",
        codec: None,
        voice: Some((
            "kokoro-voice-af_heart.gguf",
            "https://huggingface.co/cstr/kokoro-voices-GGUF/resolve/main/kokoro-voice-af_heart.gguf",
        )),
        extras: &[],
        voice_type: "ggupack",
        builtin_voices: &[
            "af_heart", "af_alloy", "af_aoede", "af_bella", "af_jessica", "af_kore",
            "af_nicole", "af_nova", "af_river", "af_sarah", "af_sky", "am_adam",
            "am_echo", "am_eric", "am_fenrir", "am_liam", "am_michael", "am_onyx",
            "am_puck", "am_santa", "bf_alice", "bf_emma", "bf_isabella", "bf_lily",
            "bm_daniel", "bm_fred", "bm_george", "bm_leo",
        ],
        supports_instruct: false,
    },
    TtsPreset {
        id: "bark",
        label: "Bark-small (мультиязычный, маркеры спикера)",
        backend: "bark",
        model_file: "bark-small-q8_0.gguf",
        model_url: "https://huggingface.co/cstr/bark-small-GGUF/resolve/main/bark-small-q8_0.gguf",
        codec: None,
        voice: None,
        extras: &[],
        voice_type: "none",
        builtin_voices: &[],
        supports_instruct: false,
    },
    TtsPreset {
        id: "zonos",
        label: "Zonos v0.1 (2B, клонирование голоса, 44.1 kHz)",
        backend: "zonos",
        model_file: "zonos-v0.1-transformer-q8_0.gguf",
        model_url:
            "https://huggingface.co/cstr/zonos-v0.1-transformer-GGUF/resolve/main/zonos-v0.1-transformer-q8_0.gguf",
        codec: Some((
            "dac-44khz-f16.gguf",
            "https://huggingface.co/cstr/dac-44khz-GGUF/resolve/main/dac-44khz-f16.gguf",
        )),
        voice: None,
        extras: &[],
        voice_type: "clone",
        builtin_voices: &[],
        supports_instruct: false,
    },
    TtsPreset {
        id: "chatterbox",
        label: "Chatterbox v3 (клонирование голоса, 23 языка)",
        backend: "chatterbox",
        model_file: "chatterbox-v3-t3-q8_0.gguf",
        model_url: "https://huggingface.co/cstr/chatterbox-GGUF/resolve/main/chatterbox-v3-t3-q8_0.gguf",
        codec: Some((
            "chatterbox-v3-s3gen-q8_0.gguf",
            "https://huggingface.co/cstr/chatterbox-GGUF/resolve/main/chatterbox-v3-s3gen-q8_0.gguf",
        )),
        voice: None,
        extras: &[],
        voice_type: "clone",
        builtin_voices: &[],
        supports_instruct: false,
    },
    TtsPreset {
        id: "vibevoice-tts",
        label: "VibeVoice-Realtime 0.5B (быстрый, en/zh)",
        backend: "vibevoice-tts",
        model_file: "vibevoice-realtime-0.5b-q4_k.gguf",
        model_url:
            "https://huggingface.co/cstr/vibevoice-realtime-0.5b-GGUF/resolve/main/vibevoice-realtime-0.5b-q4_k.gguf",
        codec: None,
        voice: Some((
            "vibevoice-voice-emma.gguf",
            "https://huggingface.co/cstr/vibevoice-realtime-0.5b-GGUF/resolve/main/vibevoice-voice-emma.gguf",
        )),
        extras: &[],
        voice_type: "ggupack",
        builtin_voices: &[],
        supports_instruct: false,
    },
    TtsPreset {
        id: "cosyvoice3-tts",
        label: "CosyVoice3 0.5B (клонирование WAV, 9 яз.)",
        backend: "cosyvoice3-tts",
        model_file: "cosyvoice3-llm-q4_k.gguf",
        model_url:
            "https://huggingface.co/cstr/cosyvoice3-0.5b-2512-GGUF/resolve/main/cosyvoice3-llm-q4_k.gguf",
        codec: Some((
            "cosyvoice3-flow-q8_0.gguf",
            "https://huggingface.co/cstr/cosyvoice3-0.5b-2512-GGUF/resolve/main/cosyvoice3-flow-q8_0.gguf",
        )),
        voice: None,
        extras: &[
            (
                "cosyvoice3-campplus-f16.gguf",
                "https://huggingface.co/cstr/cosyvoice3-0.5b-2512-GGUF/resolve/main/cosyvoice3-campplus-f16.gguf",
            ),
            (
                "cosyvoice3-s3tok-f16.gguf",
                "https://huggingface.co/cstr/cosyvoice3-0.5b-2512-GGUF/resolve/main/cosyvoice3-s3tok-f16.gguf",
            ),
            (
                "cosyvoice3-hift-f16.gguf",
                "https://huggingface.co/cstr/cosyvoice3-0.5b-2512-GGUF/resolve/main/cosyvoice3-hift-f16.gguf",
            ),
            (
                "cosyvoice3-voices.gguf",
                "https://huggingface.co/cstr/cosyvoice3-0.5b-2512-GGUF/resolve/main/cosyvoice3-voices.gguf",
            ),
        ],
        voice_type: "named",
        builtin_voices: &[
            "zero_shot", "fleurs-en", "fleurs-de", "fleurs-zh", "fleurs-ja",
            "fleurs-fr", "fleurs-es", "fleurs-ko",
        ],
        supports_instruct: false,
    },
    TtsPreset {
        id: "voxcpm2-tts",
        label: "VoxCPM2 (2B, клонирование WAV, 48 kHz)",
        backend: "voxcpm2-tts",
        model_file: "voxcpm2-q4_k.gguf",
        model_url: "https://huggingface.co/cstr/voxcpm2-GGUF/resolve/main/voxcpm2-q4_k.gguf",
        codec: None,
        voice: None,
        extras: &[],
        voice_type: "none",
        builtin_voices: &[],
        supports_instruct: false,
    },
];

pub fn preset_by_id(id: &str) -> Option<&'static TtsPreset> {
    PRESETS.iter().find(|p| p.id == id)
}

pub fn preset_backend(id: &str) -> Option<&'static str> {
    preset_by_id(id).map(|p| p.backend)
}

pub fn list_presets() -> Vec<serde_json::Value> {
    PRESETS
        .iter()
        .map(|p| {
            json!({
                "id": p.id,
                "label": p.label,
                "backend": p.backend,
                "has_codec": p.codec.is_some(),
                "has_voice": p.voice.is_some(),
                "voice_type": p.voice_type,
                "builtin_voices": p.builtin_voices,
                "supports_instruct": p.supports_instruct,
            })
        })
        .collect()
}

/// Информация о доступном бинаре движка (один вариант сборки под Windows).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineBackendInfo {
    pub id: String,
    pub label: String,
    pub asset_name: String,
    pub url: String,
    pub tag: String,
}

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    assets: Vec<GhAsset>,
}

#[derive(Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
}

/// Превращает имя ассета (`crispasr-windows-x86_64-cuda-non-cuda.zip`) в уникальный
/// `slug` (`cuda-non-cuda`), отрезая префикс `crispasr[-windows-x86_64]-` и суффикс `.zip`.
fn asset_slug(name: &str) -> Option<String> {
    let n = name.to_ascii_lowercase();
    if !n.contains("windows") || !n.ends_with(".zip") {
        return None;
    }
    let stripped = n
        .strip_prefix("crispasr-windows-x86_64-")
        .or_else(|| n.strip_prefix("crispasr-"))
        .unwrap_or(&n)
        .strip_suffix(".zip")
        .unwrap_or(&n);
    let slug = stripped.trim_matches('-').to_string();
    if slug.is_empty() {
        None
    } else {
        Some(slug)
    }
}

/// Классифицирует Windows-ассет движка. Возвращает `(id, label)`.
///
/// `id` — уникальный slug ассета (используется как имя папки и ключ бэкенда),
/// поэтому два разных CUDA-билда (`cuda` и `cuda-non-cuda`) НЕ сливаются в один пункт.
/// `label` — человекочитаемая подпись; если у одной категории несколько вариантов,
/// к ней дописывается различающий суффикс в скобках (напр. `NVIDIA CUDA (GPU) [non-cuda]`).
fn classify_backend(name: &str) -> Option<(String, String)> {
    let slug = asset_slug(name)?;

    let (cat, detail) = if slug.starts_with("cpu-legacy") {
        ("CPU (legacy SSE2)", slug.strip_prefix("cpu-legacy").unwrap_or(""))
    } else if slug.starts_with("cpu") {
        ("CPU (AVX2)", slug.strip_prefix("cpu").unwrap_or(""))
    } else if slug.contains("cuda") {
        ("NVIDIA CUDA (GPU)", slug.strip_prefix("cuda").unwrap_or(""))
    } else if slug.contains("rocm") || slug.contains("hip") {
        (
            "AMD ROCm (GPU)",
            slug.strip_prefix("rocm").or_else(|| slug.strip_prefix("hip")).unwrap_or(""),
        )
    } else if slug.contains("vulkan") {
        ("Vulkan (GPU)", slug.strip_prefix("vulkan").unwrap_or(""))
    } else {
        return None;
    };

    let detail = detail.trim_matches('-');
    let label = if detail.is_empty() {
        cat.to_string()
    } else {
        format!("{cat} [{detail}]")
    };
    Some((slug, label))
}

/// Запрашивает GitHub и возвращает список доступных Windows-бинарей движка.
pub async fn engine_backends() -> Result<Vec<EngineBackendInfo>, String> {
    let client = reqwest::Client::new();
    let resp = client
        .get(RELEASE_API)
        .header("User-Agent", "SpeechLab")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("ошибка запроса релиза CrispASR: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("GitHub вернул {} для релиза", resp.status()));
    }
    let release: GhRelease = resp
        .json()
        .await
        .map_err(|e| format!("не удалось разобрать релиз: {e}"))?;
    let tag = release.tag_name.clone();
    let mut out = Vec::new();
    for a in release.assets {
        if let Some((id, label)) = classify_backend(&a.name) {
            out.push(EngineBackendInfo {
                id,
                label,
                asset_name: a.name,
                url: a.browser_download_url,
                tag: tag.clone(),
            });
        }
    }
    if out.is_empty() {
        return Err("в релизе не найдено Windows-бинарей движка".into());
    }
    Ok(out)
}

/// Папка движка относительно exe: `<exe_dir>/crispasr`.
pub fn default_engine_dir() -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("."));
    exe.parent()
        .map(|p| p.join("crispasr"))
        .unwrap_or_else(|| PathBuf::from("crispasr"))
}

/// Папка моделей относительно exe: `<exe_dir>/tts_models`.
pub fn default_models_dir() -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("."));
    exe.parent()
        .map(|p| p.join("tts_models"))
        .unwrap_or_else(|| PathBuf::from("tts_models"))
}

/// Резолвит путь к exe движка для выбранного бэкенда.
///
/// Zip движка может распаковываться как сразу в `<engine_dir>/<backend>/crispasr.exe`,
/// так и в подпапку внутри (`.../crispasr-windows-x86_64-cuda-non-cuda/crispasr.exe`).
/// Поэтому сначала проверяем плоский путь, затем рекурсивно ищем `crispasr.exe` в папке
/// бэкенда (см. `find_exe`). Если ничего не нашли — возвращаем плоский путь для понятной
/// ошибки «движок не найден по пути: ...».
pub fn resolve_engine_exe(engine_dir: &str, backend_id: &str) -> PathBuf {
    let base: PathBuf = if engine_dir.is_empty() {
        default_engine_dir()
    } else {
        PathBuf::from(engine_dir)
    };
    let folder = base.join(backend_id);
    let direct = folder.join("crispasr.exe");
    if direct.exists() {
        return direct;
    }
    if let Some(found) = find_exe(&folder) {
        return found;
    }
    direct
}

/// Возвращает сохранённую версию установленного бэкенда (из `version.txt`), если есть.
pub fn installed_engine_version(engine_dir: &str, backend_id: &str) -> Option<String> {
    let base: PathBuf = if engine_dir.is_empty() {
        default_engine_dir()
    } else {
        PathBuf::from(engine_dir)
    };
    let vf = base.join(backend_id).join("version.txt");
    std::fs::read_to_string(&vf).ok().map(|s| s.trim().to_string())
}

fn emit_progress(app: &tauri::AppHandle, kind: &str, name: &str, downloaded: u64, total: u64) {
    let _ = app.emit(
        "tts-download",
        json!({ "kind": kind, "name": name, "downloaded": downloaded, "total": total }),
    );
}

async fn download_to(
    app: &tauri::AppHandle,
    url: &str,
    dest: &Path,
    kind: &str,
) -> Result<(), String> {
    let name = dest
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("file");
    emit_progress(app, kind, name, 0, 0);

    let resp = reqwest::Client::new()
        .get(url)
        .send()
        .await
        .map_err(|e| format!("ошибка запроса {url}: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("сервер вернул {} для {url}", resp.status()));
    }
    let total = resp.content_length().unwrap_or(0);

    let mut file = tokio::fs::File::create(dest)
        .await
        .map_err(|e| format!("не удалось создать файл {}: {e}", dest.display()))?;
    let mut stream = resp.bytes_stream();
    let mut downloaded: u64 = 0;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("ошибка скачивания: {e}"))?;
        file.write_all(&chunk)
            .await
            .map_err(|e| format!("ошибка записи: {e}"))?;
        downloaded += chunk.len() as u64;
        emit_progress(app, kind, name, downloaded, total);
    }
    file.flush().await.ok();
    Ok(())
}

/// Скачивает prebuilt `crispasr.exe` (Windows, выбранный вариант) в
/// `dest_dir/<backend_id>/`, распаковывает и сохраняет `version.txt` (тег релиза).
pub async fn download_engine(
    app: &tauri::AppHandle,
    dest_dir: &str,
    backend_id: &str,
    url: &str,
    tag: &str,
) -> Result<String, String> {
    let dir: PathBuf = if dest_dir.is_empty() {
        default_engine_dir()
    } else {
        PathBuf::from(dest_dir)
    };
    let target = dir.join(backend_id);
    std::fs::create_dir_all(&target)
        .map_err(|e| format!("не удалось создать папку {}: {e}", target.display()))?;

    let zip_path = target.join("crispasr.zip");
    download_to(app, url, &zip_path, "engine").await?;

    extract_zip(&zip_path, &target)?;
    let _ = std::fs::remove_file(&zip_path);

    // Сохраняем тег релиза для последующей проверки обновлений.
    let _ = std::fs::write(target.join("version.txt"), tag);

    let exe = find_exe(&target).ok_or_else(|| {
        format!("после распаковки не найден crispasr.exe в {}", target.display())
    })?;
    Ok(exe.to_string_lossy().to_string())
}

/// Скачивает все GGUF выбранного пресета в папку `dest_dir/<preset_id>/` и возвращает
/// пути к модели / codec / voice (codec и voice — пустые строки, если не нужны).
pub async fn download_model(
    app: &tauri::AppHandle,
    preset_id: &str,
    dest_dir: &str,
) -> Result<serde_json::Value, String> {
    let preset = preset_by_id(preset_id)
        .ok_or_else(|| format!("неизвестный пресет TTS: {preset_id}"))?;

    let dest = PathBuf::from(dest_dir).join(preset_id);
    std::fs::create_dir_all(&dest)
        .map_err(|e| format!("не удалось создать папку {}: {e}", dest.display()))?;

    let model = dest.join(preset.model_file);
    download_to(app, preset.model_url, &model, "model").await?;

    let mut codec_path = String::new();
    if let Some((cf, cu)) = preset.codec {
        let p = dest.join(cf);
        download_to(app, cu, &p, "model").await?;
        codec_path = p.to_string_lossy().to_string();
    }

    let mut voice_path = String::new();
    if let Some((vf, vu)) = preset.voice {
        let p = dest.join(vf);
        download_to(app, vu, &p, "model").await?;
        voice_path = p.to_string_lossy().to_string();
    }

    for (ef, eu) in preset.extras {
        let p = dest.join(ef);
        download_to(app, eu, &p, "model").await?;
    }

    Ok(json!({
        "model": model.to_string_lossy().to_string(),
        "codec": codec_path,
        "voice": voice_path,
    }))
}

/// Возвращает для каждого пресета статус установки в `models_dir`.
pub fn list_installed_models(models_dir: &str) -> Vec<serde_json::Value> {
    let base: PathBuf = if models_dir.is_empty() {
        default_models_dir()
    } else {
        PathBuf::from(models_dir)
    };
    PRESETS
        .iter()
        .map(|p| {
            let folder = base.join(p.id);
            let has_model = folder.join(p.model_file).exists();
            let has_codec = match p.codec {
                Some((cf, _)) => folder.join(cf).exists(),
                None => true,
            };
            let has_voice = match p.voice {
                Some((vf, _)) => folder.join(vf).exists(),
                None => true,
            };
            let installed = has_model && has_codec && has_voice;
            json!({
                "id": p.id,
                "label": p.label,
                "installed": installed,
                "has_model": has_model,
                "has_codec": has_codec,
                "has_voice": has_voice,
            })
        })
        .collect()
}

fn extract_zip(zip_path: &Path, dest: &Path) -> Result<(), String> {
    let file = std::fs::File::open(zip_path)
        .map_err(|e| format!("не удалось открыть zip {}: {e}", zip_path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| format!("не удалось прочитать zip: {e}"))?;
    archive
        .extract(dest.to_path_buf())
        .map_err(|e| format!("ошибка распаковки: {e}"))?;
    Ok(())
}

/// Рекурсивно ищет `crispasr.exe` (zip может класть в подпапку).
fn find_exe(dir: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            if let Some(found) = find_exe(&p) {
                return Some(found);
            }
        } else if p
            .file_name()
            .map(|n| n == "crispasr.exe")
            .unwrap_or(false)
        {
            return Some(p);
        }
    }
    None
}

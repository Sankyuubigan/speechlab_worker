mod modules;

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use tokio::sync::Mutex;
use tauri::{AppHandle, State};
use serde_json::{json, Value};

use modules::asr::gigaam::ModelRunner;
use modules::tts::TtsEngine;
use modules::tts::download;
use modules::tts::settings::{TtsSettings, load as load_tts_settings, save as save_tts_settings};

pub struct AppState {
    pub model: Mutex<Option<Arc<ModelRunner>>>,
    pub model_dir: Mutex<String>,
    pub cancel: Arc<AtomicBool>,
    pub tts: TtsEngine,
}

fn emit_log(app: &AppHandle, msg: &str) {
    crate::modules::log::app_log(app, msg);
}

#[tauri::command]
async fn set_model_dir(state: State<'_, AppState>, dir: String) -> Result<(), String> {
    let mut current_dir = state.model_dir.lock().await;
    if *current_dir != dir {
        *current_dir = dir;
        // Сбрасываем модель только если путь изменился
        *state.model.lock().await = None;
    }
    Ok(())
}

#[tauri::command]
async fn load_model(app: AppHandle, state: State<'_, AppState>) -> Result<String, String> {
    let dir = state.model_dir.lock().await.clone();
    if dir.is_empty() {
        emit_log(&app, "ОШИБКА: путь к модели не указан");
        return Err("путь к модели не указан".into());
    }

    emit_log(&app, &format!("Начинаю загрузку модели из {}...", dir));
    
    // Клонируем данные для передачи в spawn_blocking
    let dir_clone = dir.clone();
    let app_clone = app.clone();

    // Загрузка модели - тяжелая операция, выполняем в отдельном потоке
    let model = tokio::task::spawn_blocking(move || {
        ModelRunner::load(&dir_clone)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| {
        emit_log(&app_clone, &format!("ОШИБКА загрузки модели: {}", e));
        e.to_string()
    })?;

    *state.model.lock().await = Some(Arc::new(model));
    emit_log(&app, "Модель успешно загружена в память.");

    Ok(format!("модель успешно загружена из {dir}"))
}

    #[tauri::command]
    async fn cancel(state: State<'_, AppState>) -> Result<(), String> {
        state.cancel.store(true, Ordering::Relaxed);
        Ok(())
    }

#[tauri::command]
async fn recognize(app: AppHandle, state: State<'_, AppState>, paths: Vec<String>) -> Result<Vec<String>, String> {
    // Сбрасываем флаг отмены перед началом новой задачи
    state.cancel.store(false, Ordering::Relaxed);

    // 1. Получаем или загружаем модель
    let arc_model = {
        let mut guard = state.model.lock().await;
        if guard.is_none() {
            emit_log(&app, "Модель не загружена. Загружаю автоматически перед распознаванием...");
            let dir = state.model_dir.lock().await.clone();

            let dir_clone = dir.clone();
            let app_clone = app.clone();

            let model = tokio::task::spawn_blocking(move || {
                ModelRunner::load(&dir_clone)
            })
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| {
                emit_log(&app_clone, &format!("ОШИБКА автозагрузки модели: {}", e));
                e.to_string()
            })?;

            emit_log(&app, &format!("Автозагрузка модели завершена. Движок: {}", model.engine_name()));
            *guard = Some(Arc::new(model));
        }
        guard.as_ref().unwrap().clone()
    };

    let mut results = Vec::with_capacity(paths.len());
    let cancel = state.cancel.clone();

    // 2. Распознаем файлы
    for p in paths {
        if cancel.load(Ordering::Relaxed) {
            emit_log(&app, "⛔ Распознавание отменено пользователем.");
            results.push("[ОТМЕНЕНО]".to_string());
            continue;
        }
        emit_log(&app, &format!("--- Распознаю файл: {} ---", p));

        let p_clone = p.clone();
        let model = arc_model.clone();
        let cancel_clone = cancel.clone();

        // Само распознавание — долгий синхронный процесс (ONNX inference)
        let res = tokio::task::spawn_blocking(move || {
            model.recognize_file(&p_clone, &cancel_clone)
        })
        .await
        .map_err(|e| e.to_string())?;

        if cancel.load(Ordering::Relaxed) {
            emit_log(&app, "⛔ Распознавание отменено пользователем.");
            results.push("[ОТМЕНЕНО]".to_string());
            continue;
        }

        match res {
            Ok(text) => {
                if text.is_empty() {
                    emit_log(&app, &format!("ПУСТО ({}): модель не вернула текст — проверь аудио/препроцессор", p));
                } else {
                    emit_log(&app, &format!("УСПЕХ ({}): {}", p, text));
                }
                results.push(text);
            }
            Err(e) => {
                emit_log(&app, &format!("ОШИБКА ({}): {}", p, e));
                results.push(format!("[ОШИБКА: {}]", e));
            }
        }
    }

    emit_log(&app, "✅ Все файлы обработаны.");
    Ok(results)
}

/// Результат синтеза: WAV-байты + время генерации (сек) для отображения в UI.
#[derive(serde::Serialize)]
struct TtsSpeakResult {
    wav: Vec<u8>,
    seconds: f64,
}

/// Длительность WAV в секундах по заголовку (приблизительно, игнорируем вложенные
/// чанки). Нужна для оценки скорости синтеза (RTF) в логах.
fn wav_duration_secs(wav: &[u8]) -> Option<f64> {
    if wav.len() < 44 {
        return None;
    }
    let sr = u32::from_le_bytes([wav[24], wav[25], wav[26], wav[27]]) as f64;
    let ch = u16::from_le_bytes([wav[22], wav[23]]) as f64;
    let bits = u16::from_le_bytes([wav[34], wav[35]]) as f64;
    if sr <= 0.0 || ch <= 0.0 || bits <= 0.0 {
        return None;
    }
    let data_bytes = (wav.len() - 44) as f64;
    Some(data_bytes / (sr * ch * (bits / 8.0)))
}

#[tauri::command]
async fn tts_speak(
    app: AppHandle,
    state: State<'_, AppState>,
    preset: String,
    voice: String,
    instruct: String,
    speed: f32,
    text: String,
) -> Result<TtsSpeakResult, String> {
    let start = Instant::now();
    let settings = load_tts_settings(&app);
    let backend_id = if settings.engine_backend.is_empty() {
        "cpu".to_string()
    } else {
        settings.engine_backend.clone()
    };
    let engine_exe = download::resolve_engine_exe(&settings.engine_dir, &backend_id);
    let engine_exe_str = engine_exe.to_string_lossy().to_string();
    let backend = download::preset_backend(&preset)
        .unwrap_or("qwen3-tts")
        .to_string();

    let preset_def = download::preset_by_id(&preset);
    let voice_type = preset_def
        .map(|p| p.voice_type.clone())
        .unwrap_or_else(|| "none".to_string());
    let supports_instruct = preset_def.map(|p| p.supports_instruct).unwrap_or(false);

    let backend_clone = crate::modules::tts::clone::clone_reference_limits(&backend).is_some();
    let mut clone_ref_text = String::new();

    // Резолвим `voice` в одну из ситуаций (без копирования файлов в хранилище):
    //  - путь к WAV/GGUF-файлу → клонируем из этого файла напрямую;
    //  - bare-id, у которого есть <voice-dir>/<id>.wav → клонируем из хранилища
    //    (только для backend-ов, умеющих клонировать из WAV, иначе это
    //    зарегистрированный именованный голос);
    //  - bare-id без файла (встроенный/baked спикер) → шлём имя в теле запроса.
    let voice_input = voice.trim();
    let looks_path = !voice_input.is_empty()
        && (voice_input.contains('/')
            || voice_input.contains('\\')
            || Path::new(voice_input).is_absolute()
            || voice_input.to_ascii_lowercase().ends_with(".wav")
            || voice_input.to_ascii_lowercase().ends_with(".gguf"));

    let mut clone_source: Option<(String, String)> = None; // (исходный wav, id для темпа)
    let mut named_voice = String::new();

    if looks_path {
        if !Path::new(voice_input).exists() {
            let e = format!("файл голоса не найден: {voice_input}");
            emit_log(&app, &format!("ТТС ОШИБКА: {e}"));
            return Err(e);
        }
        clone_source = Some((
            voice_input.to_string(),
            crate::modules::tts::ascii_voice_name(voice_input),
        ));
    } else if !voice_input.is_empty() {
        let wav = crate::modules::tts::voices::voices_root(&settings.models_dir)
            .join(&voice_input)
            .join("voice.wav");
        if wav.exists() {
            if backend_clone {
                // Клонирующий backend: грузим референс из хранилища.
                clone_source = Some((wav.to_string_lossy().to_string(), voice_input.to_string()));
            } else {
                // Именованный backend: это зарегистрированный голос (customvoice).
                named_voice = voice_input.to_string();
            }
        } else {
            // Встроенный/baked спикер (fleurs-*, zero_shot и т.п.).
            named_voice = voice_input.to_string();
        }
    }

    let use_clone = backend_clone && clone_source.is_some();

    let startup_voice = if voice_type == "ggupack" {
        let base = if settings.models_dir.is_empty() {
            download::default_models_dir()
        } else {
            std::path::PathBuf::from(&settings.models_dir)
        };
        if let Some(p) = preset_def {
            if let Some((vf, _)) = &p.voice {
                let vp = base.join(&preset).join(vf);
                if vp.exists() {
                    vp.to_string_lossy().to_string()
                } else {
                    String::new()
                }
            } else {
                String::new()
            }
        } else {
            String::new()
        }
    } else if use_clone {
        // Референс грузим как `--voice` при старте сервера: cosyvoice3/zonos/…
        // игнорируют per-request `voice=<name>.wav` и ищут только baked-банку
        // (отсюда `voice ... not found (have 8)`). Стартовый `--voice <wav>`
        // задаёт клон-референс по умолчанию, и в теле запроса `voice` не шлём.
        let (src, id) = clone_source.unwrap();
        let cr = crate::modules::tts::clone::prepare_clone_reference(
            &settings.models_dir,
            &src,
            &id,
            &backend,
        )
        .map_err(|e| {
            emit_log(&app, &format!("ТТС ОШИБКА: {e}"));
            e
        })?;
        clone_ref_text = cr.ref_text;
        cr.voice_path
    } else {
        String::new()
    };

    // Сохраняем текущие настройки TTS, чтобы они пережили перезапуск приложения.
    let mut s = settings;
    s.preset = preset.clone();
    s.engine_backend = backend_id.clone();
    let _ = save_tts_settings(&app, &s);

    emit_log(&app, "ТТС: подготовка движка CrispASR...");
    state
        .tts
        .ensure(&app, &engine_exe_str, &backend, &s.models_dir, &preset, &startup_voice)
        .await
        .map_err(|e| {
            emit_log(&app, &format!("ТТС ОШИБКА: {e}"));
            e
        })?;

    // Голос для тела запроса — только для named (ggupack/WAV-clone уже загружены
    // при старте через --voice). Резолвим в bare-id без разделителей путей.
    let body_voice = if voice_type == "ggupack" || use_clone {
        String::new()
    } else {
        named_voice.clone()
    };
    let body_instruct = if supports_instruct && !instruct.is_empty() {
        instruct.clone()
    } else {
        String::new()
    };

    // Для WAV-clone бэкендов референс уже подхвачен через `--voice`; для
    // именованных (qwen3-tts-customvoice и т.п.) регистрируем голос в сервере.
    let server_voice = if voice_type == "ggupack" || body_voice.is_empty() {
        String::new()
    } else {
        state
            .tts
            .ensure_voice(&app, &s.models_dir, &body_voice)
            .await
            .map_err(|e| {
                emit_log(&app, &format!("ТТС ОШИБКА: {e}"));
                e
            })?
    };

    emit_log(&app, "ТТС: синтез речи...");
    let wav = state
        .tts
        .speak(&text, &server_voice, &body_instruct, &clone_ref_text, speed)
        .await
        .map_err(|e| {
            emit_log(&app, &format!("ТТС ОШИБКА: {e}"));
            e
        })?;

    let secs = start.elapsed().as_secs_f64();
    let rtf = wav_duration_secs(&wav).map(|d| d / secs);
    let mut msg = format!(
        "ТТС: синтез завершён за {:.2} с ({} байт WAV)",
        secs,
        wav.len()
    );
    if let Some(r) = rtf {
        msg.push_str(&format!(", скорость {:.2}x реального времени", r));
    }
    emit_log(&app, &msg);
    Ok(TtsSpeakResult {
        wav,
        seconds: secs,
    })
}

#[tauri::command]
async fn tts_presets() -> Result<Vec<Value>, String> {
    Ok(modules::tts::download::list_presets())
}

#[tauri::command]
async fn tts_unload(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    emit_log(&app, "ТТС: выгрузка движка (освобождение VRAM)...");
    state.tts.stop().await;
    Ok(())
}

/// Сохраняет синтезированный WAV (байты из фронта) по выбранному пользователем пути.
#[tauri::command]
async fn tts_save_wav(path: String, data: Vec<u8>) -> Result<(), String> {
    std::fs::write(&path, &data)
        .map_err(|e| format!("не удалось сохранить WAV в {path}: {e}"))?;
    Ok(())
}

#[tauri::command]
async fn tts_download_engine(
    app: AppHandle,
    backend_id: String,
    dest: String,
) -> Result<String, String> {
    emit_log(&app, "ТТС: получаю список бинарей CrispASR...");
    let backends = download::engine_backends().await.map_err(|e| {
        emit_log(&app, &format!("ТТС ОШИБКА: {e}"));
        e
    })?;
    let b = backends
        .iter()
        .find(|x| x.id == backend_id)
        .ok_or_else(|| format!("неизвестный бэкенд движка: {backend_id}"))?;
    emit_log(
        &app,
        &format!("ТТС: скачивание движка CrispASR ({}) в {}...", b.label, dest),
    );
    let path = download::download_engine(&app, &dest, &backend_id, &b.url, &b.tag)
        .await
        .map_err(|e| {
            emit_log(&app, &format!("ТТС ОШИБКА загрузки движка: {e}"));
            e
        })?;
    let mut s = load_tts_settings(&app);
    s.engine_dir = dest.clone();
    s.engine_backend = backend_id.clone();
    let _ = save_tts_settings(&app, &s);
    emit_log(&app, &format!("ТТС: движок скачан: {path}"));
    Ok(path)
}

#[tauri::command]
async fn tts_download_model(
    app: AppHandle,
    preset: String,
    dest: String,
) -> Result<Value, String> {
    emit_log(&app, &format!("ТТС: скачивание GGUF модели ({preset}) в {dest}..."));
    let res = download::download_model(&app, &preset, &dest)
        .await
        .map_err(|e| {
            emit_log(&app, &format!("ТТС ОШИБКА загрузки модели: {e}"));
            e
        })?;
    let mut s = load_tts_settings(&app);
    s.models_dir = dest.clone();
    s.preset = preset.clone();
    let _ = save_tts_settings(&app, &s);
    emit_log(&app, "ТТС: модель скачана");
    Ok(res)
}

#[tauri::command]
async fn tts_engine_backends() -> Result<Vec<download::EngineBackendInfo>, String> {
    download::engine_backends().await
}

#[tauri::command]
async fn tts_list_models(models_dir: String) -> Vec<Value> {
    download::list_installed_models(&models_dir)
}

#[tauri::command]
async fn tts_list_voices(models_dir: String) -> Vec<modules::tts::voices::VoiceInfo> {
    modules::tts::voices::list_voices(&models_dir)
}

#[tauri::command]
async fn tts_add_voice(
    app: AppHandle,
    models_dir: String,
    name: String,
    src_audio: String,
    ref_text: String,
    avatar: String,
    denoise: bool,
    denoise_strength: f32,
) -> Result<modules::tts::voices::VoiceInfo, String> {
    modules::tts::voices::add_voice(&app, &models_dir, &name, &src_audio, &ref_text, &avatar, denoise, denoise_strength)
}

#[tauri::command]
async fn tts_delete_voice(models_dir: String, id: String) -> Result<(), String> {
    modules::tts::voices::delete_voice(&models_dir, &id)
}

#[tauri::command]
async fn tts_update_voice(
    app: AppHandle,
    models_dir: String,
    id: String,
    name: String,
    ref_text: String,
    avatar: String,
    src_audio: String,
    denoise: bool,
    denoise_strength: f32,
) -> Result<modules::tts::voices::VoiceInfo, String> {
    modules::tts::voices::update_voice(&app, &models_dir, &id, &name, &ref_text, &avatar, &src_audio, denoise, denoise_strength)
}

#[tauri::command]
async fn tts_voice_avatar(models_dir: String, id: String) -> Option<Vec<u8>> {
    modules::tts::voices::voice_avatar(&models_dir, &id)
}

#[tauri::command]
async fn tts_voice_audio(models_dir: String, id: String) -> Result<Vec<u8>, String> {
    modules::tts::voices::voice_audio(&models_dir, &id)
}

#[tauri::command]
async fn tts_voice_trimmed_audio(
    models_dir: String,
    id: String,
    backend: String,
) -> Result<Vec<u8>, String> {
    modules::tts::clone::voice_trimmed_audio(&models_dir, &id, &backend)
}

#[tauri::command]
async fn tts_check_update(app: AppHandle) -> Value {
    let settings = load_tts_settings(&app);
    match download::engine_backends().await {
        Ok(backends) => {
            let tag = backends.first().map(|b| b.tag.clone()).unwrap_or_default();
            let engines: Vec<Value> = backends
                .iter()
                .map(|b| {
                    let installed = download::resolve_engine_exe(&settings.engine_dir, &b.id).exists();
                    let iv = download::installed_engine_version(&settings.engine_dir, &b.id);
                    let update_available = iv.as_ref().map(|v| v != &tag).unwrap_or(false);
                    json!({
                        "id": b.id,
                        "label": b.label,
                        "installed": installed,
                        "installed_version": iv,
                        "latest_version": tag,
                        "update_available": update_available,
                    })
                })
                .collect();
            json!({ "ok": true, "latest": tag, "engines": engines })
        }
        Err(e) => json!({ "ok": false, "error": e }),
    }
}

#[tauri::command]
async fn tts_default_dirs() -> Value {
    json!({
        "engine_dir": download::default_engine_dir().to_string_lossy(),
        "models_dir": download::default_models_dir().to_string_lossy(),
    })
}

#[tauri::command]
fn tts_get_settings(app: AppHandle) -> TtsSettings {
    load_tts_settings(&app)
}

#[tauri::command]
fn tts_save_settings(app: AppHandle, settings: TtsSettings) -> Result<(), String> {
    save_tts_settings(&app, &settings)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|_app| {
            crate::modules::log::truncate_last_logs();
            // Зачистка зомби от предыдущих крашей (rules.md §6.5).
            crate::modules::process_util::kill_active_engines();
            Ok(())
        })
        .manage(AppState {
            model: Mutex::new(None),
            model_dir: Mutex::new(String::new()),
            cancel: Arc::new(AtomicBool::new(false)),
            tts: TtsEngine::new(),
        })
        .invoke_handler(tauri::generate_handler![
            set_model_dir,
            load_model,
            recognize,
            cancel,
            tts_speak,
            tts_unload,
            tts_save_wav,
            tts_download_engine,
            tts_download_model,
            tts_presets,
            tts_engine_backends,
            tts_list_models,
            tts_list_voices,
            tts_add_voice,
            tts_delete_voice,
            tts_update_voice,
            tts_voice_avatar,
            tts_voice_audio,
            tts_voice_trimmed_audio,
            tts_check_update,
            tts_default_dirs,
            tts_get_settings,
            tts_save_settings
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app, event| {
            // Финальный сейфнет: если Drop не успел (насильственное закрытие),
            // добиваем все висящие движки глобально.
            if let tauri::RunEvent::ExitRequested { .. } = event {
                crate::modules::process_util::kill_active_engines();
            }
        });
}
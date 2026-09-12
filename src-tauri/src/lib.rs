mod modules;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Mutex;
use tauri::{AppHandle, Manager, State};

use modules::asr::gigaam::ModelRunner;
use tauri_plugin_speech::SttSettings;

pub struct AppState {
    pub model: Mutex<Option<Arc<ModelRunner>>>,
    pub model_dir: Mutex<String>,
    pub cancel: Arc<AtomicBool>,
    /// Channel sender for STT hotkey events (push-to-talk lifecycle).
    /// Runs in a dedicated thread; Tauri commands signal through this.
    pub stt_tx: Mutex<Option<tokio::sync::mpsc::Sender<modules::stt::hotkey::HotkeyEvent>>>,
    /// When true, HotkeyListener emits ALL key events (not just the configured hotkey).
    /// Used for hotkey assignment: user sees every keypress in real time.
    pub stt_capture_all: Arc<AtomicBool>,
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

// ─── STT: System-wide voice input commands ───────────────────────────
//
// Движковая часть STT (spawn/stop/status/ws_port) делегируется плагину
// `tauri-plugin-speech` (см. `PluginState.stt`). Здесь остаётся только
// оверлей-оркестрация: hotkey, mic, WebSocket-стриминг, tray-события.

#[tauri::command]
async fn stt_get_settings() -> SttSettings {
    SttSettings::load()
}

#[tauri::command]
async fn stt_save_settings(app: AppHandle, settings: SttSettings) -> Result<(), String> {
    settings.save()?;
    crate::modules::log::app_log(
        &app,
        &format!(
            "[stt] горячая клавиша сохранена: {} (code {})",
            settings.hotkey_name, settings.hotkey_code
        ),
    );
    Ok(())
}

#[tauri::command]
async fn stt_start(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<u16, String> {
    let settings = SttSettings::load();
    // Движок STT живёт в плагине (PluginState.stt). Хост отвечает только за
    // оверлей: hotkey-цикл, микрофон, WebSocket-стриминг в движок.
    let ws_port = app
        .state::<tauri_plugin_speech::PluginState>()
        .stt
        .ensure(&app, &settings)
        .await?;

    let mut hotkey = modules::stt::hotkey::HotkeyListener::new(settings.hotkey_code, state.stt_capture_all.clone());
    let mut rx = hotkey.start(&app);

    let app_clone = app.clone();
    let hotkey_code = settings.hotkey_code;

    tokio::spawn(async move {
        let mut recording = false;
        let mut stop_tx: Option<tokio::sync::oneshot::Sender<()>> = None;

        while let Some(evt) = rx.recv().await {
            match evt {
                modules::stt::hotkey::HotkeyEvent::Press => {
                    if !recording {
                        recording = true;
                        modules::stt::tray::emit_status(&app_clone, &modules::stt::SttStatus::Recording);
                        modules::stt::tray::emit_key_log(&app_clone, &modules::stt::hotkey::code_to_name(hotkey_code), hotkey_code, true);

                        let (tx, stop) = tokio::sync::oneshot::channel::<()>();
                        stop_tx = Some(tx);

                        let app_rec = app_clone.clone();
                        // cpal::Stream is !Send, so we use std::thread::spawn instead of tokio::spawn.
                        // Communication with mic thread is via channels.
                        std::thread::spawn(move || {
                            let mut mic = match modules::stt::mic::MicCapture::new() {
                                Ok(m) => m,
                                Err(e) => { modules::stt::tray::emit_status(&app_rec, &modules::stt::SttStatus::Error(e)); return; }
                            };
                            let (audio_tx, mut audio_rx) = tokio::sync::mpsc::channel::<Vec<f32>>(64);
                            if let Err(e) = mic.start(audio_tx) {
                                modules::stt::tray::emit_status(&app_rec, &modules::stt::SttStatus::Error(e));
                                return;
                            }
                            let mut ws = modules::stt::ws_client::SttWsClient::new(ws_port);
                            if let Err(e) = ws.connect() {
                                modules::stt::tray::emit_status(&app_rec, &modules::stt::SttStatus::Error(e));
                                return;
                            }
                            // Block on channel recv + ws using a local tokio runtime
                            let rt = tokio::runtime::Builder::new_current_thread()
                                .enable_all()
                                .build()
                                .unwrap();
                            rt.block_on(async {
                                let mut stop = stop;
                                loop {
                                    tokio::select! {
                                        chunk = audio_rx.recv() => {
                                            match chunk {
                                                Some(samples) => {
                                                    let _ = ws.send_audio(&samples);
                                                    while let Some(result) = ws.recv_result() {
                                                        modules::stt::tray::emit_result(&app_rec, &result.text, result.is_final);
                                                    }
                                                }
                                                None => break,
                                            }
                                        }
                                        _ = &mut stop => { break; }
                                    }
                                }
                            });
                            drop(mic);
                            std::thread::sleep(std::time::Duration::from_millis(500));
                            while let Some(result) = ws.recv_result() {
                                modules::stt::tray::emit_result(&app_rec, &result.text, result.is_final);
                            }
                            ws.disconnect();
                        });
                    }
                }
                modules::stt::hotkey::HotkeyEvent::Release => {
                    if recording {
                        recording = false;
                        modules::stt::tray::emit_status(&app_clone, &modules::stt::SttStatus::Transcribing);
                        modules::stt::tray::emit_key_log(&app_clone, &modules::stt::hotkey::code_to_name(hotkey_code), hotkey_code, false);
                        stop_tx.take();
                        tokio::time::sleep(std::time::Duration::from_millis(800)).await;
                        modules::stt::tray::emit_status(&app_clone, &modules::stt::SttStatus::Listening);
                    }
                }
            }
        }
    });

    Ok(ws_port)
}

#[tauri::command]
async fn stt_stop(
    app: AppHandle,
    _state: State<'_, AppState>,
) -> Result<(), String> {
    app.state::<tauri_plugin_speech::PluginState>().stt.stop(&app).await;
    Ok(())
}

#[tauri::command]
async fn stt_get_status(app: AppHandle) -> Result<String, String> {
    Ok(match app.state::<tauri_plugin_speech::PluginState>().stt.status() {
        tauri_plugin_speech::SttStatus::Stopped => "stopped",
        tauri_plugin_speech::SttStatus::Starting => "starting",
        tauri_plugin_speech::SttStatus::Listening => "listening",
        tauri_plugin_speech::SttStatus::Recording => "recording",
        tauri_plugin_speech::SttStatus::Transcribing => "transcribing",
        tauri_plugin_speech::SttStatus::Error(_) => "error",
    }
    .to_string())
}

#[tauri::command]
async fn stt_inject_text(text: String) -> Result<(), String> {
    modules::stt::inject::inject_text(&text)
}

// ─── Main entry ──────────────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_speech::init())
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
            stt_tx: Mutex::new(None),
            stt_capture_all: Arc::new(AtomicBool::new(false)),
        })
        .invoke_handler(tauri::generate_handler![
            set_model_dir,
            load_model,
            recognize,
            cancel,
            stt_get_settings,
            stt_save_settings,
            stt_start,
            stt_stop,
            stt_get_status,
            stt_inject_text,
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
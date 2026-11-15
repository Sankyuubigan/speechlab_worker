use std::time::{Duration, Instant};

use serde_json::json;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_speech::DialogSettings;

use super::playback::AudioPlayer;
use super::ws_client::{DialogServerEvent, DialogWsClient, MIC_SAMPLE_RATE};
use crate::modules::audio::mic::MicCapture;

/// Таймаут ожидания `started` после подключения (включает первый прогрев
/// модели ~6.6 GB с диска).
const STARTED_TIMEOUT: Duration = Duration::from_secs(300);

/// Сколько ждать `end` после hang_up, прежде чем закрыть соединение.
const END_DRAIN: Duration = Duration::from_secs(3);

/// Статус сессии для фронтенда (`dialog://status`).
pub fn emit_status(app: &AppHandle, state: &str, detail: &str) {
    let _ = app.emit(
        "dialog://status",
        json!({"state": state, "detail": detail}),
    );
}

/// Сборка start-JSON по настройкам (SSOT полей — `DialogSettings` плагина:
/// движковые поля model/port/engine_exe сюда не входят).
fn start_message(settings: &DialogSettings) -> serde_json::Value {
    let mut start = json!({"type": "start", "sample_rate": MIC_SAMPLE_RATE});
    if !settings.language.is_empty() && settings.language != "auto" {
        start["language"] = json!(settings.language);
    }
    if !settings.name.is_empty() {
        start["name"] = json!(settings.name);
    }
    if !settings.persona.is_empty() {
        start["persona"] = json!(settings.persona);
    }
    if !settings.voice.is_empty() {
        start["voice"] = json!(settings.voice);
    }
    if settings.seed != 0 {
        start["seed"] = json!(settings.seed);
    }
    if settings.speak_first {
        start["speak_first"] = json!(true);
    }
    if settings.max_seconds > 0 {
        start["max_seconds"] = json!(settings.max_seconds);
    }
    start
}

/// Ручка управления активной сессией: dialog_stop шлёт сигнал в поток сессии.
pub struct DialogSessionHandle {
    stop: tokio::sync::oneshot::Sender<()>,
}

impl DialogSessionHandle {
    pub fn stop(self) {
        let _ = self.stop.send(());
    }
}

/// Запускает поток сессии диалога (mic → WS, WS → playback + события).
/// Возвращает ручку немедленно; тяжёлая работа — в отдельном потоке
/// (cpal::Stream !Send, как и в STT).
pub fn start_dialog(
    app: AppHandle,
    ws_port: u16,
    settings: DialogSettings,
) -> Result<DialogSessionHandle, String> {
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let app_cleanup = app.clone();
    std::thread::spawn(move || {
        run_session(app, ws_port, settings, stop_rx);
        // Сессия закончилась (любым исходом) — сбрасываем ручку в состоянии,
        // чтобы dialog_start мог начать новую.
        if let Some(st) = app_cleanup.try_state::<crate::AppState>() {
            *st.dialog.blocking_lock() = None;
        }
    });
    Ok(DialogSessionHandle { stop: stop_tx })
}

fn run_session(
    app: AppHandle,
    ws_port: u16,
    settings: DialogSettings,
    stop: tokio::sync::oneshot::Receiver<()>,
) {
    emit_status(&app, "connecting", "");

    let mut ws = DialogWsClient::new(ws_port);
    if let Err(e) = ws.connect(&start_message(&settings)) {
        log::info!("[dialog] ошибка подключения: {e}");
        emit_status(&app, "error", &e);
        return;
    }

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("локальный runtime для сессии диалога");

    rt.block_on(async move {
        // ── 1. Ждём `started` (прогрев модели может занять минуты) ──
        let deadline = Instant::now() + STARTED_TIMEOUT;
        let (sample_rate, frame_samples) = loop {
            match ws.recv() {
                Some(DialogServerEvent::Started {
                    sample_rate,
                    frame_samples,
                }) => break (sample_rate, frame_samples),
                Some(DialogServerEvent::Error(e)) => {
                    log::info!("[dialog] сервер сообщил об ошибке до старта: {e}");
                    emit_status(&app, "error", &e);
                    return;
                }
                _ => {}
            }
            if Instant::now() > deadline {
                let msg = format!("таймаут ожидания started ({STARTED_TIMEOUT:?})");
                log::info!("[dialog] {msg}");
                emit_status(&app, "error", &msg);
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        };
        log::info!(
            "[dialog] сессия началась: rate={sample_rate}, кадр={frame_samples} сэмплов"
        );

        // ── 2. Плеер на rate сервера ──
        let mut player = match AudioPlayer::new(sample_rate) {
            Ok(p) => p,
            Err(e) => {
                log::info!("[dialog] {e}");
                emit_status(&app, "error", &e);
                return;
            }
        };

        // ── 3. Микрофон (после старта: аудио до `started` сервер не ждёт) ──
        let mut mic = match MicCapture::new() {
            Ok(m) => m,
            Err(e) => {
                log::info!("[dialog] микрофон: {e}");
                emit_status(&app, "error", &e);
                return;
            }
        };
        let (audio_tx, mut audio_rx) = tokio::sync::mpsc::channel::<Vec<f32>>(64);
        if let Err(e) = mic.start_with_rate(audio_tx, MIC_SAMPLE_RATE) {
            log::info!("[dialog] микрофон: {e}");
            emit_status(&app, "error", &e);
            return;
        }

        emit_status(&app, "active", "");

        // ── 4. Основной цикл: mic → WS, WS → playback/события ──
        let mut stop = stop;
        let mut finished = false;
        loop {
            tokio::select! {
                chunk = audio_rx.recv() => {
                    match chunk {
                        Some(samples) => {
                            if let Err(e) = ws.send_pcm(&samples) {
                                log::info!("[dialog] отправка микрофона прервана: {e}");
                                emit_status(&app, "error", &e);
                                break;
                            }
                        }
                        None => break,
                    }
                }
                _ = &mut stop => {
                    log::info!("[dialog] получен сигнал остановки, отправляю hang_up");
                    ws.hang_up();
                    // Дожидаемся `end` со статистикой, но не вечно.
                    let deadline = Instant::now() + END_DRAIN;
                    while Instant::now() < deadline {
                        match ws.recv() {
                            Some(DialogServerEvent::End { frames, samples }) => {
                                log::info!("[dialog] разговор завершён: frames={frames}, samples={samples}");
                                break;
                            }
                            Some(DialogServerEvent::Error(e)) => {
                                log::info!("[dialog] ошибка при завершении: {e}");
                                break;
                            }
                            _ => {}
                        }
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                    break;
                }
            }

            // Разбор событий сервера (накопившиеся за время итерации).
            while let Some(ev) = ws.recv() {
                match ev {
                    DialogServerEvent::Text(t) => {
                        let _ = app.emit("dialog://text", json!({"text": t}));
                    }
                    DialogServerEvent::Audio(bytes) => player.enqueue_pcm(&bytes),
                    DialogServerEvent::End { frames, samples } => {
                        log::info!("[dialog] разговор завершён сервером: frames={frames}, samples={samples}");
                        finished = true;
                    }
                    DialogServerEvent::Error(e) => {
                        log::info!("[dialog] ошибка сессии: {e}");
                        emit_status(&app, "error", &e);
                        finished = true;
                    }
                    DialogServerEvent::Started { .. } => {}
                }
                if finished {
                    break;
                }
            }
            if finished {
                break;
            }
        }

        player.stop();
        mic.stop();
        ws.disconnect();
        emit_status(&app, "stopped", "");
    });
}

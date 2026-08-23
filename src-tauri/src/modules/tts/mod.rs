use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

use reqwest::Client;
use tauri::AppHandle;

use crate::modules::process_util::{kill_process_tree, JobGuard};

pub mod download;
pub mod settings;
pub mod voices;

use download::preset_by_id;

const HEALTH_TIMEOUT: Duration = Duration::from_secs(120);

/// Движок TTS на базе CrispASR.
///
/// CrispASR запускается как **отдельный prebuilt-процесс** (`crispasr.exe`) и общается
/// по HTTP localhost (OpenAI-совместимый `POST /v1/audio/speech`). Нативная линковка
/// крейта `crispasr` запрещена глобальным правилом `desktop_rust_tauri/rules.md §6.7`.
///
/// Всегда 100% локально: без API-ключей, без облака (требование пользователя).
pub struct TtsEngine {
    child: Mutex<Option<Child>>,
    /// Job Object (Windows), гарантирующий зачистку дерева процессов при выходе.
    job: Mutex<Option<JobGuard>>,
    port: Mutex<u16>,
    loaded_model: Mutex<String>,
    loaded_codec: Mutex<String>,
    loaded_voice: Mutex<String>,
    loaded_backend: Mutex<String>,
}

impl TtsEngine {
    pub fn new() -> Self {
        Self {
            child: Mutex::new(None),
            job: Mutex::new(None),
            port: Mutex::new(0),
            loaded_model: Mutex::new(String::new()),
            loaded_codec: Mutex::new(String::new()),
            loaded_voice: Mutex::new(String::new()),
            loaded_backend: Mutex::new(String::new()),
        }
    }

    /// Возвращает держит ли движок тот же набор моделей/голоса (для переиспользования).
    fn same_model(&self, model: &str, codec: &str, voice: &str, backend: &str) -> bool {
        let m = self.loaded_model.lock().unwrap();
        let c = self.loaded_codec.lock().unwrap();
        let v = self.loaded_voice.lock().unwrap();
        let b = self.loaded_backend.lock().unwrap();
        m.as_str() == model && c.as_str() == codec && v.as_str() == voice && b.as_str() == backend
    }

    fn pick_port() -> Option<u16> {
        let listener = TcpListener::bind("127.0.0.1:0").ok()?;
        let port = listener.local_addr().ok()?.port();
        Some(port)
    }

    /// Убивает движок: дёргает Job Object (весь процесс-трий целиком) и дублирует
    /// `kill_process_tree` (`taskkill /F /T` до репарентинга). `job` сбрасывается
    /// при выходе из области видимости (закрытие хендла → KILL_ON_JOB_CLOSE).
    fn kill_now(mut child: Child, job: Option<JobGuard>) {
        if let Some(j) = &job {
            j.terminate();
        }
        kill_process_tree(&mut child);
        // job (и child) корректно освобождаются здесь.
    }

    /// Берёт child+job из мьютексов (устойчиво к poison) и убивает движок.
    fn reap(&self) {
        let child = self
            .child
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        let job = self.job.lock().unwrap_or_else(|e| e.into_inner()).take();
        match child {
            Some(c) => Self::kill_now(c, job),
            None => drop(job),
        }
    }

    /// Запускает (или переиспользуется) сервер CrispASR для заданных model/codec/backend.
    ///
    /// Голос типа "ggupack" (GGUF voice-пак) грузится при старте через `--voice <путь>`,
    /// т.к. передача GGUF-пака в теле запроса сервер трактует как клонирование и требует
    /// `consent_attestation`. Голоса типа "named"/"clone" передаются в теле запроса.
    ///
    /// Логи stderr движка перенаправляются в Логи (событие `app-log` с префиксом
    /// `[crispasr]`), чтобы пользователь видел причину падения.
    pub async fn ensure(
        &self,
        app: &AppHandle,
        engine_exe: &str,
        backend: &str,
        models_dir: &str,
        preset_id: &str,
        startup_voice: &str,
    ) -> Result<(), String> {
        if engine_exe.is_empty() {
            return Err("не указан путь к движку crispasr.exe (откройте Настройки → ТТС)".into());
        }

        let preset = preset_by_id(preset_id)
            .ok_or_else(|| format!("неизвестный пресет TTS: {preset_id}"))?;

        let base = if models_dir.is_empty() {
            download::default_models_dir()
        } else {
            std::path::PathBuf::from(models_dir)
        };
        let folder = base.join(preset_id);

        let model = folder.join(&preset.model_file);
        if !model.exists() {
            return Err(format!(
                "модель не найдена: {} (скачайте пресет в Настройках)",
                model.display()
            ));
        }

        let mut codec_path = String::new();
        if let Some((cf, _)) = &preset.codec {
            let p = folder.join(cf);
            if !p.exists() {
                return Err(format!("codec-модель не найдена: {}", p.display()));
            }
            codec_path = p.to_string_lossy().to_string();
        }

        let mut startup_voice_path = String::new();
        if !startup_voice.is_empty() {
            if !std::path::Path::new(startup_voice).exists() {
                return Err(format!("файл голоса не найден: {startup_voice}"));
            }
            startup_voice_path = startup_voice.to_string();
        }

        let running = self.child.lock().unwrap().is_some();
        if running && self.same_model(
            &model.to_string_lossy(),
            &codec_path,
            &startup_voice_path,
            backend,
        ) {
            return Ok(());
        }
        if running {
            self.stop().await;
        }

        if !std::path::Path::new(engine_exe).exists() {
            return Err(format!("движок не найден по пути: {engine_exe}"));
        }

        let port = Self::pick_port().ok_or_else(|| "не удалось выбрать свободный порт".to_string())?;

        let mut cmd = Command::new(engine_exe);
        cmd.args([
            "--server",
            "--backend",
            backend,
            "-m",
            &model.to_string_lossy(),
            "--host",
            "127.0.0.1",
            "--port",
            &port.to_string(),
        ]);
        if !codec_path.is_empty() {
            cmd.args(["--codec-model", &codec_path]);
        }
        // Хранилище голосов: сервер резолвит `voice` против --voice-dir и отвергает
        // любые пути в поле `voice` (bug `400 invalid_voice`). Обязателен для клонирования.
        let voice_dir = voices::voices_root(models_dir);
        cmd.args(["--voice-dir", &voice_dir.to_string_lossy()]);
        // Голос-пак GGUF грузим при старте; именованные/клонированные — в теле запроса.
        if !startup_voice_path.is_empty() {
            cmd.args(["--voice", &startup_voice_path]);
        }
        #[cfg(windows)]
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        cmd.stdout(Stdio::null()).stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("не удалось запустить движок '{engine_exe}': {e}"))?;

        // Назначаем процесс в Job Object (Windows): гарантирует зачистку всего
        // дерева процессов даже при насильственном закрытии приложения.
        let job = JobGuard::assign(&child);

        // Перенаправляем stderr движка в Логи.
        if let Some(stderr) = child.stderr.take() {
            let app_clone = app.clone();
            std::thread::spawn(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines().flatten() {
                    crate::modules::log::app_log(&app_clone, &format!("[crispasr] {line}"));
                }
            });
        }

        // Ждём готовности сервера (без авторизации).
        let client = Client::new();
        let health = format!("http://127.0.0.1:{port}/health");
        let started = std::time::Instant::now();
        let mut ready = false;
        while started.elapsed() < HEALTH_TIMEOUT {
            if let Ok(Some(_)) = child.try_wait() {
                return Err("процесс движка завершился сразу (см. Логи: ошибка запуска/модели)".into());
            }
            if let Ok(resp) = client
                .get(&health)
                .timeout(Duration::from_secs(2))
                .send()
                .await
            {
                if resp.status().is_success() {
                    ready = true;
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        if !ready {
            Self::kill_now(child, job);
            return Err("таймаут ожидания готовности движка (120 c)".into());
        }

        *self.child.lock().unwrap() = Some(child);
        *self.job.lock().unwrap() = job;
        *self.port.lock().unwrap() = port;
        *self.loaded_model.lock().unwrap() = model.to_string_lossy().to_string();
        *self.loaded_codec.lock().unwrap() = codec_path;
        *self.loaded_voice.lock().unwrap() = startup_voice_path;
        *self.loaded_backend.lock().unwrap() = backend.to_string();
        Ok(())
    }

    /// Синтезирует текст в WAV и возвращает сырые байты.
    ///
    /// `voice` — имя спикера или путь к WAV для клонирования (передаётся в теле запроса,
    /// per-request; может быть пустым). `consent_attestation` добавляется ВСЕГДА
    /// (требование CosyVoice3 и других бэкендов — даже при пустом/дефолтном voice любой
    /// синтез трактуется как клонирование). Лишнее поле другие бэкенды игнорируют.
    /// `instructions` — стиль/описание голоса (только для поддерживающих моделей).
    pub async fn speak(
        &self,
        text: &str,
        voice: &str,
        instructions: &str,
        speed: f32,
    ) -> Result<Vec<u8>, String> {
        let port = *self.port.lock().unwrap();
        let backend = self.loaded_backend.lock().unwrap().clone();
        if port == 0 {
            return Err("движок не запущен (вызовите ensure перед speak)".into());
        }

        let client = Client::new();
        let url = format!("http://127.0.0.1:{port}/v1/audio/speech");

        let mut body = serde_json::Map::new();
        body.insert("model".into(), serde_json::Value::String(backend));
        body.insert("input".into(), serde_json::Value::String(text.to_string()));
        body.insert("response_format".into(), serde_json::Value::String("wav".into()));
        if !voice.is_empty() {
            body.insert("voice".into(), serde_json::Value::String(voice.to_string()));
        }
        // consent_attestation требуется бэкендами (напр. CosyVoice3) всегда — даже при
        // пустом/дефолтном voice любой синтез трактуется как клонирование. Безопасно для
        // остальных бэкендов (игнорируют неизвестное поле).
        body.insert(
            "consent_attestation".into(),
            serde_json::Value::String(
                "I confirm I have the legal right to clone this voice.".into(),
            ),
        );
        if !instructions.is_empty() {
            body.insert(
                "instructions".into(),
                serde_json::Value::String(instructions.to_string()),
            );
        }
        if (speed - 1.0).abs() > f32::EPSILON {
            body.insert("speed".into(), serde_json::Value::from(speed));
        }
        let body_value = serde_json::Value::Object(body);

        let resp = client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&body_value)
            .send()
            .await
            .map_err(|e| format!("ошибка запроса TTS: {e}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let detail = resp.text().await.unwrap_or_default();
            return Err(format!("TTS ошибка {status}: {detail}"));
        }

        let bytes = resp
            .bytes()
            .await
            .map_err(|e| format!("ошибка чтения ответа TTS: {e}"))?;
        Ok(bytes.to_vec())
    }

    /// Останавливает движок (выгрузка моделей из VRAM).
    pub async fn stop(&self) {
        self.reap();
        *self.loaded_model.lock().unwrap() = String::new();
        *self.loaded_codec.lock().unwrap() = String::new();
        *self.loaded_voice.lock().unwrap() = String::new();
        *self.loaded_backend.lock().unwrap() = String::new();
    }
}

impl Default for TtsEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for TtsEngine {
    fn drop(&mut self) {
        self.reap();
    }
}

use std::net::TcpStream;

use serde_json::json;
use tungstenite::protocol::Message;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{connect, WebSocket};

/// Протокол WS `/v1/dialogue` speech.cpp (SSOT констант):
/// кадр модели = 3200 сэмплов @ 24кГц = 133 мс.
pub const MODEL_SAMPLE_RATE: u32 = 24000;
/// Частота микрофона, объявляемая серверу в `start.sample_rate`.
pub const MIC_SAMPLE_RATE: u32 = 16000;

/// Событие от сервера диалога.
#[derive(Debug, Clone)]
pub enum DialogServerEvent {
    /// Ответ на start: сервер подтвердил параметры (rate модели, размер кадра).
    Started { sample_rate: u32, frame_samples: u32 },
    /// Текст реплики перед очередным аудио-кадром.
    Text(String),
    /// Бинарный аудио-кадр (16-bit LE mono PCM, MODEL_SAMPLE_RATE).
    Audio(Vec<u8>),
    /// Разговор завершён (после hang_up или по инициативе сервера).
    End { frames: u64, samples: u64 },
    /// Ошибка протокола/сервера.
    Error(String),
}

/// WebSocket-клиент диалога с speech.cpp (`ws://127.0.0.1:{port}/v1/dialogue`).
///
/// Первым сообщением клиент шлёт JSON `{"type":"start", "sample_rate": ..., ...}`,
/// затем бинарные кадры 16-bit LE mono PCM. Сервер шлёт `started`, `text` перед
/// каждым аудио-кадром, сами бинарные кадры, `end`, `error`.
pub struct DialogWsClient {
    ws: Option<WebSocket<MaybeTlsStream<TcpStream>>>,
    port: u16,
}

impl DialogWsClient {
    pub fn new(port: u16) -> Self {
        Self { ws: None, port }
    }

    /// Подключается и отправляет start-сообщение с настройками карточки.
    pub fn connect(&mut self, start: &serde_json::Value) -> Result<(), String> {
        let url = format!("ws://127.0.0.1:{}/v1/dialogue", self.port);
        let (mut ws, _) = connect(&url).map_err(|e| format!("WebSocket handshake failed: {e}"))?;
        // Неблокирующий режим — recv() опрашивает без зависания.
        match ws.get_mut() {
            MaybeTlsStream::Plain(s) => s
                .set_nonblocking(true)
                .map_err(|e| format!("failed to set nonblocking: {e}"))?,
            #[allow(unreachable_patterns)]
            _ => return Err("TLS-соединения не поддерживаются для localhost".into()),
        }
        ws.write_message(Message::Text(start.to_string().into()))
            .map_err(|e| format!("WS send start error: {e}"))?;
        log::info!("[dialog] WS подключён: {url}, start отправлен");
        self.ws = Some(ws);
        Ok(())
    }

    /// Отправляет фрагмент микрофонного звука (f32 mono) как 16-bit LE PCM.
    pub fn send_pcm(&mut self, samples: &[f32]) -> Result<(), String> {
        if let Some(ws) = &mut self.ws {
            let bytes: Vec<u8> = samples
                .iter()
                .map(|s| (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
                .flat_map(|v| v.to_le_bytes())
                .collect();
            ws.write_message(Message::Binary(bytes.into()))
                .map_err(|e| format!("WS send error: {e}"))?;
            Ok(())
        } else {
            Err("WebSocket not connected".into())
        }
    }

    /// Неблокирующее чтение события сервера; None — данных пока нет.
    pub fn recv(&mut self) -> Option<DialogServerEvent> {
        let ws = self.ws.as_mut()?;
        match ws.read_message() {
            Ok(Message::Text(text)) => Self::parse_json(&text),
            Ok(Message::Binary(data)) => Some(DialogServerEvent::Audio(data.to_vec())),
            Ok(Message::Close(frame)) => {
                let reason = frame
                    .map(|f| f.reason.to_string())
                    .unwrap_or_default();
                Some(DialogServerEvent::Error(format!("сервер закрыл соединение: {reason}")))
            }
            Ok(_) => None, // Ping/Pong/Frame
            Err(tungstenite::Error::Io(ref e))
                if e.kind() == std::io::ErrorKind::WouldBlock =>
            {
                None
            }
            Err(e) => Some(DialogServerEvent::Error(format!("WS recv error: {e}"))),
        }
    }

    fn parse_json(text: &str) -> Option<DialogServerEvent> {
        let v: serde_json::Value = serde_json::from_str(text).ok()?;
        let t = v.get("type")?.as_str()?;
        match t {
            "started" => Some(DialogServerEvent::Started {
                sample_rate: v.get("sample_rate").and_then(|x| x.as_u64()).unwrap_or(MODEL_SAMPLE_RATE as u64) as u32,
                frame_samples: v.get("frame_samples").and_then(|x| x.as_u64()).unwrap_or(3200) as u32,
            }),
            "text" => Some(DialogServerEvent::Text(
                v.get("text").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            )),
            "end" => Some(DialogServerEvent::End {
                frames: v.get("frames").and_then(|x| x.as_u64()).unwrap_or(0),
                samples: v.get("samples").and_then(|x| x.as_u64()).unwrap_or(0),
            }),
            "error" => Some(DialogServerEvent::Error(
                v.get("error")
                    .or_else(|| v.get("message"))
                    .and_then(|x| x.as_str())
                    .unwrap_or("неизвестная ошибка сервера")
                    .to_string(),
            )),
            _ => None,
        }
    }

    /// Вежливое завершение разговора (сервер пришлёт `end` со статистикой).
    pub fn hang_up(&mut self) {
        if let Some(ws) = &mut self.ws {
            let msg = json!({"type": "hang_up"});
            if let Err(e) = ws.write_message(Message::Text(msg.to_string().into())) {
                log::info!("[dialog] hang_up send error: {e}");
            }
        }
    }

    /// Закрыть соединение.
    pub fn disconnect(&mut self) {
        if let Some(mut ws) = self.ws.take() {
            let _ = ws.close(None);
        }
    }
}

impl Drop for DialogWsClient {
    fn drop(&mut self) {
        self.disconnect();
    }
}

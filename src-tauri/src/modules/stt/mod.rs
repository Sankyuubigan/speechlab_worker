pub mod hotkey;
pub mod inject;
pub mod mic;
pub mod tray;
pub mod ws_client;

/// Состояние STT-оверлея (hotkey/mic цикл). Движковая часть STT (spawn/stop/
/// status/ws_port) живёт в плагине `tauri-plugin-speech` — `PluginState.stt`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SttStatus {
    Stopped,
    Starting,
    Listening,
    Recording,
    Transcribing,
    Error(String),
}
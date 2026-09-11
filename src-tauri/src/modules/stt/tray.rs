use tauri::{AppHandle, Emitter};

use super::SttStatus;

/// Emit STT status event to frontend (for tray icon updates + GUI display).
pub fn emit_status(app: &AppHandle, status: &SttStatus) {
    let label = match status {
        SttStatus::Stopped => "stopped",
        SttStatus::Starting => "starting",
        SttStatus::Listening => "listening",
        SttStatus::Recording => "recording",
        SttStatus::Transcribing => "transcribing",
        SttStatus::Error(_) => "error",
    };
    let _ = app.emit("stt-status", label);
}

/// Emit recognized text to frontend.
pub fn emit_result(app: &AppHandle, text: &str, is_final: bool) {
    let _ = app.emit(
        "stt-result",
        serde_json::json!({ "text": text, "final": is_final }),
    );
}

/// Emit key press log to frontend (for hotkey configuration).
pub fn emit_key_log(app: &AppHandle, key_name: &str, key_code: u32, pressed: bool) {
    let _ = app.emit(
        "stt-key",
        serde_json::json!({
            "name": key_name,
            "code": key_code,
            "pressed": pressed,
        }),
    );
}

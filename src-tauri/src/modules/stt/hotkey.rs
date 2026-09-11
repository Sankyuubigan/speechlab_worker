use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::thread;

use rdev::{listen, Event, EventType, Key};
use tauri::{AppHandle, Emitter};

/// Push-to-talk state machine.
///
/// Listens for global key events. When the configured hotkey is pressed,
/// signals "recording start". When released, signals "recording stop".
///
/// When `capture_all` is true, ALL key events are emitted as `stt-key` events
/// for hotkey discovery — the user sees every keypress in real time.
pub struct HotkeyListener {
    running: Arc<AtomicBool>,
    hotkey_code: Arc<AtomicU32>,
    capture_all: Arc<AtomicBool>,
    press_tx: Option<tokio::sync::mpsc::Sender<HotkeyEvent>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyEvent {
    Press,
    Release,
}

impl HotkeyListener {
    pub fn new(hotkey_code: u32, capture_all: Arc<AtomicBool>) -> Self {
        Self {
            running: Arc::new(AtomicBool::new(false)),
            hotkey_code: Arc::new(AtomicU32::new(hotkey_code)),
            capture_all,
            press_tx: None,
        }
    }

    /// Start listening in a background thread. Returns a receiver for hotkey events.
    pub fn start(
        &mut self,
        app: &AppHandle,
    ) -> tokio::sync::mpsc::Receiver<HotkeyEvent> {
        let (tx, rx) = tokio::sync::mpsc::channel(32);
        self.press_tx = Some(tx.clone());

        let running = self.running.clone();
        let hotkey_code = self.hotkey_code.clone();
        let capture_all = self.capture_all.clone();
        let app = app.clone();

        running.store(true, Ordering::SeqCst);

        thread::spawn(move || {
            let callback = move |event: Event| {
                if !running.load(Ordering::SeqCst) {
                    return;
                }
                let event_code = match key_to_code(event.event_type) {
                    Some(c) => c,
                    None => return,
                };

                // capture_all mode: emit ALL key events for hotkey discovery
                if capture_all.load(Ordering::SeqCst) {
                    let (name, pressed) = match event.event_type {
                        EventType::KeyPress(k) => (key_name(k), true),
                        EventType::KeyRelease(k) => (key_name(k), false),
                        _ => return,
                    };
                    let _ = app.emit(
                        "stt-key",
                        serde_json::json!({
                            "name": name,
                            "code": event_code,
                            "pressed": pressed,
                        }),
                    );
                    return;
                }

                // Normal mode: only forward events matching the configured hotkey
                let target_code = hotkey_code.load(Ordering::SeqCst);
                if event_code == target_code {
                    let evt = match event.event_type {
                        EventType::KeyPress(_) => Some(HotkeyEvent::Press),
                        EventType::KeyRelease(_) => Some(HotkeyEvent::Release),
                        _ => None,
                    };
                    if let Some(e) = evt {
                        let _ = tx.blocking_send(e);
                    }
                }
            };

            if let Err(e) = listen(callback) {
                eprintln!("[stt] rdev listen error: {e:?}");
            }
        });

        rx
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    pub fn update_hotkey(&self, new_code: u32) {
        self.hotkey_code.store(new_code, Ordering::SeqCst);
    }
}

/// Convert rdev Key to a stable numeric code for persistence.
pub fn key_to_code(et: EventType) -> Option<u32> {
    match et {
        EventType::KeyPress(k) | EventType::KeyRelease(k) => Some(key_to_u32(k)),
        _ => None,
    }
}

pub fn key_to_u32(k: Key) -> u32 {
    match k {
        Key::Backspace => 0x08,
        Key::Tab => 0x09,
        Key::Return => 0x0D,
        Key::Escape => 0x1B,
        Key::Space => 0x20,
        Key::Delete => 0x2E,
        Key::Insert => 0x2D,
        Key::Home => 0x24,
        Key::End => 0x23,
        Key::PageUp => 0x21,
        Key::PageDown => 0x22,
        Key::LeftArrow => 0x25,
        Key::UpArrow => 0x26,
        Key::RightArrow => 0x27,
        Key::DownArrow => 0x28,
        Key::F1 => 0x70,
        Key::F2 => 0x71,
        Key::F3 => 0x72,
        Key::F4 => 0x73,
        Key::F5 => 0x74,
        Key::F6 => 0x75,
        Key::F7 => 0x76,
        Key::F8 => 0x77,
        Key::F9 => 0x78,
        Key::F10 => 0x79,
        Key::F11 => 0x7A,
        Key::F12 => 0x7B,
        Key::KeyA => 0x41,
        Key::KeyB => 0x42,
        Key::KeyC => 0x43,
        Key::KeyD => 0x44,
        Key::KeyE => 0x45,
        Key::KeyF => 0x46,
        Key::KeyG => 0x47,
        Key::KeyH => 0x48,
        Key::KeyI => 0x49,
        Key::KeyJ => 0x4A,
        Key::KeyK => 0x4B,
        Key::KeyL => 0x4C,
        Key::KeyM => 0x4D,
        Key::KeyN => 0x4E,
        Key::KeyO => 0x4F,
        Key::KeyP => 0x50,
        Key::KeyQ => 0x51,
        Key::KeyR => 0x52,
        Key::KeyS => 0x53,
        Key::KeyT => 0x54,
        Key::KeyU => 0x55,
        Key::KeyV => 0x56,
        Key::KeyW => 0x57,
        Key::KeyX => 0x58,
        Key::KeyY => 0x59,
        Key::KeyZ => 0x5A,
        Key::Num0 => 0x30,
        Key::Num1 => 0x31,
        Key::Num2 => 0x32,
        Key::Num3 => 0x33,
        Key::Num4 => 0x34,
        Key::Num5 => 0x35,
        Key::Num6 => 0x36,
        Key::Num7 => 0x37,
        Key::Num8 => 0x38,
        Key::Num9 => 0x39,
        Key::PrintScreen => 0x2C,
        Key::ScrollLock => 0x91,
        Key::Pause => 0x13,
        Key::CapsLock => 0x14,
        Key::Alt => 0xA2,
        Key::AltGr => 0xA4,
        Key::ShiftLeft => 0xA0,
        Key::ShiftRight => 0xA1,
        Key::ControlLeft => 0xA3,
        Key::ControlRight => 0xA5,
        Key::MetaLeft => 0x5B,
        Key::MetaRight => 0x5C,
        Key::Dot => 0xBE,
        Key::Minus => 0xBD,
        Key::Equal => 0xBB,
        Key::BackQuote => 0xC0,
        Key::LeftBracket => 0xDB,
        Key::BackSlash => 0xDC,
        Key::RightBracket => 0xDD,
        Key::Quote => 0xDE,
        Key::SemiColon => 0xBA,
        Key::Comma => 0xBC,
        Key::Slash => 0xBF,
        _ => 0xFFFF, // unknown
    }
}

pub fn key_name(k: Key) -> String {
    code_to_name(key_to_u32(k))
}

pub fn code_to_name(code: u32) -> String {
    match code {
        0x08 => "Backspace",
        0x09 => "Tab",
        0x0D => "Enter",
        0x1B => "Escape",
        0x20 => "Space",
        0x2E => "Delete",
        0x2D => "Insert",
        0x24 => "Home",
        0x23 => "End",
        0x21 => "PageUp",
        0x22 => "PageDown",
        0x25 => "Left",
        0x26 => "Up",
        0x27 => "Right",
        0x28 => "Down",
        0x70..=0x7B => return format!("F{}", code - 0x6F),
        0x41..=0x5A => return ((b'A' + (code - 0x41) as u8) as char).to_string(),
        0x30..=0x39 => return ((b'0' + (code - 0x30) as u8) as char).to_string(),
        0x2C => "PrintScreen",
        0x91 => "ScrollLock",
        0x13 => "Pause",
        0x14 => "CapsLock",
        0xA2 => "LAlt",
        0xA4 => "RAlt",
        0xA0 => "LShift",
        0xA1 => "RShift",
        0xA3 => "LCtrl",
        0xA5 => "RCtrl",
        0x5B => "LWin",
        0x5C => "RWin",
        0xBE => ".",
        0xBD => "-",
        0xBB => "=",
        0xC0 => "`",
        0xDB => "[",
        0xDC => "\\",
        0xDD => "]",
        0xDE => "'",
        0xBA => ";",
        0xBC => ",",
        0xBF => "/",
        _ => "Unknown",
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_name_roundtrip() {
        assert_eq!(code_to_name(0x41), "A");
        assert_eq!(code_to_name(0x70), "F1");
        assert_eq!(code_to_name(0x2E), "Delete");
    }
}

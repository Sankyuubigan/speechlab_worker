use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleFormat, StreamConfig};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

const TARGET_SAMPLE_RATE: u32 = 16000;

fn log_info(msg: &str) {
    eprintln!("{msg}");
}

/// Microphone capture: records 16kHz mono float32 audio chunks.
///
/// Usage:
///   let mut mic = MicCapture::new()?;
///   mic.start(tx)?;
///   // ... recording happens, audio sent via channel ...
///   mic.stop();
pub struct MicCapture {
    running: Arc<AtomicBool>,
    stream: Option<cpal::Stream>,
}

impl MicCapture {
    pub fn new() -> Result<Self, String> {
        Ok(Self {
            running: Arc::new(AtomicBool::new(false)),
            stream: None,
        })
    }

    /// Start capturing microphone audio. Sends Vec<f32> (mono 16kHz) chunks via channel.
    /// Each chunk is ~100ms of audio (1600 samples).
    pub fn start(&mut self, tx: tokio::sync::mpsc::Sender<Vec<f32>>) -> Result<(), String> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or_else(|| "no microphone found".to_string())?;

        let supported = device
            .default_input_config()
            .map_err(|e| format!("failed to get mic config: {e}"))?;

        let sample_rate = supported.sample_rate().0;
        let channels = supported.channels() as usize;
        let sample_format = supported.sample_format();

        log_info(&format!(
            "[stt] mic: device='{}', {channels}ch@{sample_rate}Hz, fmt={sample_format:?}",
            device.name().unwrap_or_default()
        ));

        // We need 16kHz mono float32 output
        let config = StreamConfig {
            channels: 1,
            sample_rate: cpal::SampleRate(TARGET_SAMPLE_RATE),
            buffer_size: cpal::BufferSize::Default,
        };

        let running = self.running.clone();
        running.store(true, Ordering::SeqCst);

        // Buffer for accumulating samples before sending
        let chunk_size = (TARGET_SAMPLE_RATE / 10) as usize; // 100ms chunks = 1600 samples
        let buffer = Arc::new(Mutex::new(Vec::<f32>::with_capacity(chunk_size * 2)));

        let buffer_clone = buffer.clone();
        let running_clone = running.clone();

        let stream = match sample_format {
            SampleFormat::F32 => {
                let buffer = buffer_clone;
                let running = running_clone;
                device
                    .build_input_stream(
                        &config,
                        move |data: &[f32], _: &cpal::InputCallbackInfo| {
                            if !running.load(Ordering::SeqCst) {
                                return;
                            }
                            let mut buf = buffer.lock().unwrap();
                            // Resample if source rate != 16kHz, or just take mono channel
                            if channels == 1 {
                                buf.extend_from_slice(data);
                            } else {
                                // Take first channel only
                                for (i, sample) in data.iter().enumerate() {
                                    if i % channels == 0 {
                                        buf.push(*sample);
                                    }
                                }
                            }
                            while buf.len() >= chunk_size {
                                let chunk: Vec<f32> = buf.drain(..chunk_size).collect();
                                let _ = tx.blocking_send(chunk);
                            }
                        },
                        |e| {
                            eprintln!("[stt] mic stream error: {e}");
                        },
                        None,
                    )
                    .map_err(|e| format!("failed to build mic stream: {e}"))?
            }
            SampleFormat::I16 => {
                let buffer = buffer_clone;
                let running = running_clone;
                device
                    .build_input_stream(
                        &config,
                        move |data: &[i16], _: &cpal::InputCallbackInfo| {
                            if !running.load(Ordering::SeqCst) {
                                return;
                            }
                            let mut buf = buffer.lock().unwrap();
                            for (i, sample) in data.iter().enumerate() {
                                if i % channels == 0 {
                                    buf.push(*sample as f32 / i16::MAX as f32);
                                }
                            }
                            while buf.len() >= chunk_size {
                                let chunk: Vec<f32> = buf.drain(..chunk_size).collect();
                                let _ = tx.blocking_send(chunk);
                            }
                        },
                        |e| {
                            eprintln!("[stt] mic stream error: {e}");
                        },
                        None,
                    )
                    .map_err(|e| format!("failed to build mic stream: {e}"))?
            }
            SampleFormat::U16 => {
                let buffer = buffer_clone;
                let running = running_clone;
                device
                    .build_input_stream(
                        &config,
                        move |data: &[u16], _: &cpal::InputCallbackInfo| {
                            if !running.load(Ordering::SeqCst) {
                                return;
                            }
                            let mut buf = buffer.lock().unwrap();
                            for (i, sample) in data.iter().enumerate() {
                                if i % channels == 0 {
                                    buf.push((*sample as f32 / u16::MAX as f32) * 2.0 - 1.0);
                                }
                            }
                            while buf.len() >= chunk_size {
                                let chunk: Vec<f32> = buf.drain(..chunk_size).collect();
                                let _ = tx.blocking_send(chunk);
                            }
                        },
                        |e| {
                            eprintln!("[stt] mic stream error: {e}");
                        },
                        None,
                    )
                    .map_err(|e| format!("failed to build mic stream: {e}"))?
            }
            _ => return Err(format!("unsupported sample format: {sample_format:?}")),
        };

        stream
            .play()
            .map_err(|e| format!("failed to start mic stream: {e}"))?;

        self.stream = Some(stream);
        Ok(())
    }

    pub fn stop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        self.stream.take(); // Drop stops the stream
    }
}

impl Drop for MicCapture {
    fn drop(&mut self) {
        self.stop();
    }
}

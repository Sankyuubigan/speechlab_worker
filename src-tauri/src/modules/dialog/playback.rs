use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleFormat, StreamConfig};

/// Стартовый джиттер-буфер: не начинаем потребление, пока в очереди меньше
/// двух кадров модели (2 × 3200 сэмплов @ 24кГц ≈ 266 мс).
const PREBUFFER_SAMPLES: usize = 6400;

/// Проигрыватель голоса Haru: очередь сэмплов (24кГц mono) → cpal output.
///
/// Устройство вывода обычно работает на своей частоте (48кГц WASAPI shared),
/// поэтому в колбэке делаем линейную интерполяцию src→dst. При недодаче данных
/// играем тишину и «встаём на паузу» по времени — догоняем без искажений.
pub struct AudioPlayer {
    queue: Arc<Mutex<VecDeque<f32>>>,
    running: Arc<AtomicBool>,
    stream: Option<cpal::Stream>,
}

impl AudioPlayer {
    /// Создаёт поток вывода на дефолтном устройстве. `src_rate` — частота
    /// входных сэмплов (см. `started.sample_rate` от сервера).
    pub fn new(src_rate: u32) -> Result<Self, String> {
        let host = cpal::default_host();
        let device: Device = host
            .default_output_device()
            .ok_or_else(|| "аудиоустройство вывода не найдено".to_string())?;

        let supported = device
            .default_output_config()
            .map_err(|e| format!("не удалось получить конфиг вывода: {e}"))?;
        let dst_rate = supported.sample_rate().0;
        let channels = supported.channels() as usize;
        let sample_format = supported.sample_format();
        if sample_format != SampleFormat::F32 {
            return Err(format!(
                "неподдерживаемый формат вывода: {sample_format:?} (ожидается F32)"
            ));
        }

        let config = StreamConfig {
            channels: channels as u16,
            sample_rate: cpal::SampleRate(dst_rate),
            buffer_size: cpal::BufferSize::Default,
        };

        let queue = Arc::new(Mutex::new(VecDeque::<f32>::new()));
        // Дробная позиция чтения в «виртуальном» потоке сэмплов источника
        // относительно начала очереди (1.0 = один сэмпл источника).
        let pos = Arc::new(Mutex::new(0.0f64));
        let started = Arc::new(AtomicBool::new(false));
        let running = Arc::new(AtomicBool::new(true));
        let step = f64::from(src_rate) / f64::from(dst_rate);

        let queue_cb = queue.clone();
        let pos_cb = pos.clone();
        let started_cb = started.clone();
        let running_cb = running.clone();

        let stream = device
            .build_output_stream(
                &config,
                move |out: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    if !running_cb.load(Ordering::SeqCst) {
                        out.fill(0.0);
                        return;
                    }
                    let Ok(mut q) = queue_cb.lock() else {
                        out.fill(0.0);
                        return;
                    };
                    // Джиттер-буфер: копим стартовый запас перед первым звуком.
                    if !started_cb.load(Ordering::SeqCst) {
                        if q.len() < PREBUFFER_SAMPLES {
                            out.fill(0.0);
                            return;
                        }
                        started_cb.store(true, Ordering::SeqCst);
                    }
                    let Ok(mut p) = pos_cb.lock() else {
                        out.fill(0.0);
                        return;
                    };
                    for frame in out.chunks_mut(channels) {
                        let idx = *p as usize;
                        // Недодача данных: тишина, время замирает (догоним позже).
                        if idx + 1 >= q.len() {
                            frame.fill(0.0);
                            continue;
                        }
                        let s0 = q[idx];
                        let s1 = q[idx + 1];
                        let frac = (*p - idx as f64) as f32;
                        let sample = s0 + (s1 - s0) * frac;
                        for slot in frame.iter_mut() {
                            *slot = sample;
                        }
                        *p += step;
                        while *p >= 1.0 {
                            q.pop_front();
                            *p -= 1.0;
                        }
                    }
                },
                |e| eprintln!("[dialog] output stream error: {e}"),
                None,
            )
            .map_err(|e| format!("не удалось создать поток вывода: {e}"))?;

        stream
            .play()
            .map_err(|e| format!("не удалось запустить вывод: {e}"))?;

        Ok(Self {
            queue,
            running,
            stream: Some(stream),
        })
    }

    /// Добавляет бинарный аудио-кадр сервера (16-bit LE mono) в очередь.
    pub fn enqueue_pcm(&self, bytes: &[u8]) {
        if let Ok(mut q) = self.queue.lock() {
            q.reserve(bytes.len() / 2);
            for pair in bytes.chunks_exact(2) {
                let v = i16::from_le_bytes([pair[0], pair[1]]);
                q.push_back(v as f32 / i16::MAX as f32);
            }
        }
    }

    pub fn stop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        self.stream.take(); // Drop останавливает поток
    }
}

impl Drop for AudioPlayer {
    fn drop(&mut self) {
        self.stop();
    }
}

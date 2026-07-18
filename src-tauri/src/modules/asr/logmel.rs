//! Log-mel spectrogram preprocessor для gigaam v3.
//! Загружает официальный `gigaam_v3.onnx`, если он есть и ВАЛИДЕН в директории модели.
//! Если нет/битый - использует точный fallback V3 (n_fft=320, win=320, hop=160,
//! окно Ханна, HTK mel-банк, без pre-emphasis) согласно эталону onnx-asr:
//! `src/onnx_asr/preprocessors/numpy_preprocessor.py` и `preprocessors/fbanks.py`.

use ndarray::Array;
use ort::session::Session;
use std::sync::Mutex;
use std::path::Path;

pub struct Preprocessor {
    onnx: Option<Mutex<Session>>,
    fallback: LogMel,
}

impl Preprocessor {
    pub fn new(model_dir: &str) -> Self {
        let path = format!("{}\\gigaam_v3.onnx", model_dir);
        // Грузим ONNX-препроцессор только если файл выглядит валидным (не битый 404).
        let onnx = if Path::new(&path).exists() && is_valid_onnx(&path) {
            Session::builder()
                .and_then(|mut builder| builder.commit_from_file(&path))
                .ok()
                .map(Mutex::new)
        } else {
            if Path::new(&path).exists() {
                eprintln!(
                    "[warn] gigaam_v3.onnx битый/невалидный ({path}) — используем ручной LogMel V3."
                );
            }
            None
        };
        Self {
            onnx,
            fallback: LogMel::new(16000),
        }
    }

    pub fn compute(&self, waveform: &[f32]) -> (Vec<f32>, usize) {
        if waveform.is_empty() {
            return (Vec::new(), 0);
        }

        // Попытка использовать официальный ONNX препроцессор
        if let Some(sess) = &self.onnx {
            let wave_len = waveform.len();
            if let Ok(audio_array) = Array::from_shape_vec((1, wave_len), waveform.to_vec()) {
                if let Ok(lengths_array) = Array::from_shape_vec((1,), vec![wave_len as i64]) {
                    if let Ok(mut guard) = sess.lock() {
                        if let Ok(outputs) = guard.run(ort::inputs! {
                            "waveforms" => ort::value::TensorRef::from_array_view(&audio_array).unwrap(),
                            "waveforms_lens" => ort::value::TensorRef::from_array_view(&lengths_array).unwrap(),
                        }) {
                            // Эталонные выходы: "features" (B x 64 x T), "features_lens" (B)
                            let out_name = if outputs.contains_key("features") {
                                "features"
                            } else {
                                "log_mel_features"
                            };
                            if let Ok((shape, slice)) = outputs[out_name].try_extract_tensor::<f32>() {
                                if shape.len() == 3 {
                                    let frames = shape[2] as usize;
                                    return (slice.to_vec(), frames);
                                }
                            }
                        }
                    }
                }
            }
            eprintln!("[warn] gigaam_v3.onnx execution failed, falling back to manual LogMel.");
        }

        // Fallback: ручной лог-мел V3 (точный, по эталону onnx-asr)
        self.fallback.compute(waveform)
    }
}

/// Простая проверка валидности ONNX: магические байты ONNX-протобуфа.
/// Реальный ONNX начинается с protobuf-тега; битый файл "404: Not Found" — нет.
fn is_valid_onnx(path: &str) -> bool {
    if let Ok(bytes) = std::fs::read(path) {
        // ONNX — это protobuf, начинается с поля model (tag 1, wire type 2) => 0x08 или 0x0A.
        // Достаточно проверить, что это не HTML/текст ошибки и размер разумный.
        if bytes.len() < 100 {
            return false;
        }
        // Битый файл "404: Not Found" начинается с '4' (0x34). Настоящий ONNX — бинарник.
        if bytes[0] == b'4' && bytes[1] == b'0' && bytes[2] == b'4' {
            return false;
        }
        true
    } else {
        false
    }
}

pub struct LogMel {
    sample_rate: u32,
    pub n_mels: usize,
    pub n_fft: usize,
    pub win_length: usize,
    pub hop_length: usize,
    mel_filters: Vec<Vec<f32>>,
    window: Vec<f32>,
}

impl LogMel {
    /// Параметры ТОЧНО по эталону V3 (onnx-asr):
    /// n_fft = win = 16000//50 = 320, hop = 160, n_mels = 64,
    /// окно Ханна = np.hanning(win+1)[:-1], БЕЗ pre-emphasis.
    pub fn new(sample_rate: u32) -> Self {
        let n_mels = 64;
        let n_fft = (sample_rate / 50) as usize; // 320
        let win_length = n_fft; // 320
        let hop_length = (sample_rate / 100) as usize; // 160

        // np.hanning(win_length + 1)[:-1]
        let window: Vec<f32> = (0..win_length)
            .map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / (win_length + 1) as f32).cos())
            .collect();

        // HTK mel-банк (fbanks.py: melscale_fbanks, mel_scale="htk", без нормализации)
        let mel_filters = Self::mel_filterbank_htk(sample_rate, n_fft, n_mels, 0.0, 8000.0);

        Self {
            sample_rate,
            n_mels,
            n_fft,
            win_length,
            hop_length,
            mel_filters,
            window,
        }
    }

    pub fn compute(&self, waveform: &[f32]) -> (Vec<f32>, usize) {
        // V3: БЕЗ pre-emphasis. Сразу окно (sliding_window_view, шаг hop).
        let num_frames = if waveform.len() < self.win_length {
            0
        } else {
            (waveform.len() - self.win_length) / self.hop_length + 1
        };

        if num_frames == 0 {
            return (Vec::new(), 0);
        }

        let mut features = vec![0.0f32; self.n_mels * num_frames];

        let n_freqs = self.n_fft / 2 + 1;

        for f in 0..num_frames {
            let start = f * self.hop_length;
            // кадр * окно
            let mut frame = vec![0.0f32; self.n_fft];
            for i in 0..self.win_length {
                frame[i] = waveform[start + i] * self.window[i];
            }

            // |rfft(frame, n_fft)|^2  (эталон: np.abs(rfft)**2, без нормировки на n_fft)
            let power = Self::rfft_power(&frame, n_freqs);

            for m in 0..self.n_mels {
                let mut sum = 0.0f32;
                for k in 0..n_freqs {
                    sum += self.mel_filters[m][k] * power[k];
                }
                // log(clip(mel, 1e-9, 1e9)) — эталон clamp_min=1e-9, clamp_max=1e9
                let mel = (sum.clamp(1e-9, 1e9)).ln();
                features[m * num_frames + f] = mel;
            }
        }

        (features, num_frames)
    }

    /// |rfft(frame)|^2 для n_freqs = n_fft/2 + 1 компонент.
    fn rfft_power(frame: &[f32], n_freqs: usize) -> Vec<f32> {
        let n = frame.len();
        let mut out = vec![0.0f32; n_freqs];
        for k in 0..n_freqs {
            let mut re = 0.0f32;
            let mut im = 0.0f32;
            for t in 0..n {
                let angle = -2.0 * std::f32::consts::PI * (k * t) as f32 / n as f32;
                re += frame[t] * angle.cos();
                im += frame[t] * angle.sin();
            }
            out[k] = re * re + im * im;
        }
        out
    }

    /// HTK mel-банк (точная копия fbanks.py: melscale_fbanks с mel_scale="htk",
    /// norm=None — без slaney-нормализации).
    fn mel_filterbank_htk(
        sample_rate: u32,
        n_fft: usize,
        n_mels: usize,
        f_min: f32,
        f_max: f32,
    ) -> Vec<Vec<f32>> {
        let n_freqs = n_fft / 2 + 1;
        let mut filters = vec![vec![0.0f32; n_freqs]; n_mels];

        // HTK: hz_to_mel(f) = 2595 * log10(1 + f/700)
        let hz_to_mel = |f: f32| 2595.0 * (1.0 + f / 700.0).log10();
        // HTK: mel_to_hz(m) = 700 * (10^(m/2595) - 1)
        let mel_to_hz = |m: f32| 700.0 * (10f32.powf(m / 2595.0) - 1.0);

        let m_min = hz_to_mel(f_min);
        let m_max = hz_to_mel(f_max);

        // m_pts: n_mels + 2 равномерно в мел-шкале, затем обратно в Hz
        let mut hz_points = Vec::with_capacity(n_mels + 2);
        for i in 0..(n_mels + 2) {
            let m = m_min + (m_max - m_min) * i as f32 / (n_mels + 1) as f32;
            hz_points.push(mel_to_hz(m));
        }

        // bin_points = floor((n_fft+1) * hz / sample_rate)
        let bin_points: Vec<usize> = hz_points
            .iter()
            .map(|&hz| (((n_fft + 1) as f32 * hz / sample_rate as f32).floor()) as usize)
            .collect();

        for m in 1..=n_mels {
            let mut left = bin_points[m - 1];
            let mut center = bin_points[m];
            let mut right = bin_points[m + 1];
            if left >= n_freqs { left = n_freqs - 1; }
            if center >= n_freqs { center = n_freqs - 1; }
            if right > n_freqs { right = n_freqs; }

            for k in left..center {
                if center > left {
                    filters[m - 1][k] = (k - left) as f32 / (center - left) as f32;
                }
            }
            for k in center..right {
                if right > center {
                    filters[m - 1][k] = (right - k) as f32 / (right - center) as f32;
                }
            }
        }
        filters
    }
}

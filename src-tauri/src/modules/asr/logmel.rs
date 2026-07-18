//! Log-mel spectrogram preprocessor для gigaam v3.
//! Загружает официальный `gigaam_v3.onnx`, если он есть и ВАЛИДЕН в директории модели.
//! Если нет/битый - использует точный fallback V3 (n_fft=320, win=320, hop=160,
//! окно Ханна, точный PyTorch/Torchaudio mel-банк, без pre-emphasis).

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

        if let Some(sess) = &self.onnx {
            let wave_len = waveform.len();
            if let Ok(audio_array) = Array::from_shape_vec((1, wave_len), waveform.to_vec()) {
                if let Ok(lengths_array) = Array::from_shape_vec((1,), vec![wave_len as i64]) {
                    if let Ok(mut guard) = sess.lock() {
                        if let Ok(outputs) = guard.run(ort::inputs! {
                            "waveforms" => ort::value::TensorRef::from_array_view(&audio_array).unwrap(),
                            "waveforms_lens" => ort::value::TensorRef::from_array_view(&lengths_array).unwrap(),
                        }) {
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

        self.fallback.compute(waveform)
    }
}

fn is_valid_onnx(path: &str) -> bool {
    if let Ok(bytes) = std::fs::read(path) {
        if bytes.len() < 100 {
            return false;
        }
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
    pub fn new(sample_rate: u32) -> Self {
        let n_mels = 64;
        let n_fft = (sample_rate / 50) as usize; // 320
        let win_length = n_fft; // 320
        let hop_length = (sample_rate / 100) as usize; // 160

        // Окно Ханна: точно совпадает с PyTorch `hann_window(periodic=True)`
        // и `numpy.hanning(win_length + 1)[:-1]` — делить нужно строго на win_length.
        let window: Vec<f32> = (0..win_length)
            .map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / win_length as f32).cos())
            .collect();

        // Точный фильтрбанк на основе Torchaudio (с интерполяцией вещественных частот)
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
            let mut frame = vec![0.0f32; self.n_fft];
            for i in 0..self.win_length {
                frame[i] = waveform[start + i] * self.window[i];
            }

            let power = Self::rfft_power(&frame, n_freqs);

            for m in 0..self.n_mels {
                let mut sum = 0.0f32;
                for k in 0..n_freqs {
                    sum += self.mel_filters[m][k] * power[k];
                }
                let mel = (sum.clamp(1e-9, 1e9)).ln();
                features[m * num_frames + f] = mel;
            }
        }

        (features, num_frames)
    }

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

    /// Полноценный Torchaudio/Librosa-совместимый HTK Mel-фильтрбанк.
    /// Старая реализация использовала округление бинов (индексов), из-за чего 
    /// нижние частоты полностью искажались. Этот метод применяет точную линейную 
    /// интерполяцию по вещественной частотной сетке, как в эталонном Python.
    fn mel_filterbank_htk(
        sample_rate: u32,
        n_fft: usize,
        n_mels: usize,
        f_min: f32,
        f_max: f32,
    ) -> Vec<Vec<f32>> {
        let n_freqs = n_fft / 2 + 1;
        let mut filters = vec![vec![0.0f32; n_freqs]; n_mels];

        let hz_to_mel = |f: f32| 2595.0 * (1.0 + f / 700.0).log10();
        let mel_to_hz = |m: f32| 700.0 * (10f32.powf(m / 2595.0) - 1.0);

        let m_min = hz_to_mel(f_min);
        let m_max = hz_to_mel(f_max);

        // Центральные частоты для треугольных фильтров
        let mut f_pts = Vec::with_capacity(n_mels + 2);
        for i in 0..(n_mels + 2) {
            let m = m_min + (m_max - m_min) * i as f32 / (n_mels + 1) as f32;
            f_pts.push(mel_to_hz(m));
        }

        // Частоты STFT
        let mut all_freqs = Vec::with_capacity(n_freqs);
        for i in 0..n_freqs {
            all_freqs.push((sample_rate / 2) as f32 * i as f32 / (n_freqs - 1) as f32);
        }

        for m in 0..n_mels {
            let f_left = f_pts[m];
            let f_center = f_pts[m + 1];
            let f_right = f_pts[m + 2];

            for k in 0..n_freqs {
                let freq = all_freqs[k];
                let down_slope = (freq - f_left) / (f_center - f_left);
                let up_slope = (f_right - freq) / (f_right - f_center);
                // Треугольный фильтр
                filters[m][k] = down_slope.min(up_slope).max(0.0);
            }
        }
        filters
    }
}
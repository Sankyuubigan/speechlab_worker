//! Log-mel spectrogram preprocessor для gigaam v3.
//! Загружает официальный `gigaam_v3.onnx`, если он есть в директории модели.
//! Если нет - использует точный fallback (n_fft=512, win=1024, hop=160, pre-emphasis=0.97).

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
        let onnx = if Path::new(&path).exists() {
            Session::builder()
                .and_then(|mut builder| builder.commit_from_file(&path))
                .ok()
                .map(Mutex::new)
        } else {
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
                            "audio_signal" => ort::value::TensorRef::from_array_view(&audio_array).unwrap(),
                            "length" => ort::value::TensorRef::from_array_view(&lengths_array).unwrap(),
                        }) {
                            if let Ok((shape, slice)) = outputs["log_mel_features"].try_extract_tensor::<f32>() {
                                // shape должно быть [1, 64, frames]
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

        // Fallback: ручной лог-мел
        self.fallback.compute(waveform)
    }
}

pub struct LogMel {
    pub sample_rate: u32,
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
        let n_fft = 512;
        let win_length = 1024;
        let hop_length = 160;

        let window: Vec<f32> = (0..win_length)
            .map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / win_length as f32).cos())
            .collect();

        let mel_filters = Self::mel_filterbank(sample_rate, n_fft, n_mels);

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
        let mut emph = vec![0.0f32; waveform.len()];
        if !waveform.is_empty() {
            emph[0] = waveform[0];
            for i in 1..waveform.len() {
                emph[i] = waveform[i] - 0.97 * waveform[i - 1];
            }
        }

        let num_frames = if emph.len() < self.win_length {
            0
        } else {
            (emph.len() - self.win_length) / self.hop_length + 1
        };

        if num_frames == 0 {
            return (Vec::new(), 0);
        }

        let mut features = vec![0.0f32; self.n_mels * num_frames];

        for f in 0..num_frames {
            let start = f * self.hop_length;
            let mut frame = vec![0.0f32; self.win_length];
            for i in 0..self.win_length {
                frame[i] = emph[start + i] * self.window[i];
            }
            
            // Time-domain aliasing (wrap), так как win_length > n_fft
            let mut wrapped = vec![0.0f32; self.n_fft];
            for i in 0..self.win_length {
                wrapped[i % self.n_fft] += frame[i];
            }

            let power = Self::rfft_power(&wrapped);
            
            for m in 0..self.n_mels {
                let mut sum = 0.0f32;
                for (k, &p) in power.iter().enumerate() {
                    sum += self.mel_filters[m][k] * p;
                }
                let mel = (sum + 1e-10).ln();
                features[m * num_frames + f] = mel;
            }
        }

        (features, num_frames)
    }

    fn rfft_power(frame: &[f32]) -> Vec<f32> {
        let n = frame.len();
        let half = n / 2 + 1;
        let mut out = vec![0.0f32; half];
        for k in 0..half {
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

    fn mel_filterbank(sample_rate: u32, n_fft: usize, n_mels: usize) -> Vec<Vec<f32>> {
        let n_freqs = n_fft / 2 + 1;
        let mut filters = vec![vec![0.0f32; n_freqs]; n_mels];

        let hz_to_mel = |f: f32| 2595.0 * (1.0 + f / 700.0).ln();
        let mel_to_hz = |m: f32| 700.0 * (m.exp() - 1.0);

        let f_min = 0.0f32;
        let f_max = 8000.0f32;
        let low_mel = hz_to_mel(f_min);
        let high_mel = hz_to_mel(f_max);
        let mut mel_points = Vec::with_capacity(n_mels + 2);
        for i in 0..(n_mels + 2) {
            mel_points.push(low_mel + (high_mel - low_mel) * i as f32 / (n_mels + 1) as f32);
        }
        let hz_points: Vec<f32> = mel_points.iter().map(|&m| mel_to_hz(m)).collect();
        let bin_points: Vec<usize> = hz_points
            .iter()
            .map(|&hz| ((n_fft as f32 + 1.0) * hz / sample_rate as f32).floor() as usize)
            .collect();

        for m in 1..=n_mels {
            let mut left = bin_points[m - 1];
            let mut center = bin_points[m];
            let mut right = bin_points[m + 1];
            if left >= n_freqs { left = n_freqs - 1; }
            if center >= n_freqs { center = n_freqs - 1; }
            if right > n_freqs { right = n_freqs; }
            for k in left..center {
                if center > left { filters[m - 1][k] = (k - left) as f32 / (center - left) as f32; }
            }
            for k in center..right {
                if right > center { filters[m - 1][k] = (right - k) as f32 / (right - center) as f32; }
            }
        }
        filters
    }
}
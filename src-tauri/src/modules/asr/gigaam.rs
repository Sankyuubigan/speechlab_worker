use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use anyhow::{Context, Result};
use ort::session::Session;
use std::sync::Mutex;

use crate::modules::audio::decode_to_mono;
use crate::modules::asr::logmel::Preprocessor;

/// Обертка, позволяющая загрузить любую из двух моделей (CTC или RNNT).
/// Универсально: работает с любыми найденными файлами gigaam в папке.
pub enum ModelRunner {
    Ctc(GigaamCtc),
    Rnnt(GigaamRnnt),
}

impl ModelRunner {
    pub fn load(model_dir: &str) -> Result<Self> {
        let rnnt_enc = format!("{model_dir}\\v3_e2e_rnnt_encoder.int8.onnx");
        let ctc_mod = format!("{model_dir}\\v3_e2e_ctc.int8.onnx");

        // Авто-детект по наличию файлов. Приоритет RNNT (точнее), если есть оба.
        let have_rnnt = Path::new(&rnnt_enc).exists()
            || find_file_opt(model_dir, "rnnt_encoder").is_some();
        let have_ctc = Path::new(&ctc_mod).exists()
            || find_file_opt(model_dir, "ctc").is_some();

        if have_rnnt {
            Ok(Self::Rnnt(GigaamRnnt::load(model_dir)?))
        } else if have_ctc {
            Ok(Self::Ctc(GigaamCtc::load(model_dir)?))
        } else {
            anyhow::bail!("Не найдены подходящие файлы onnx (ctc/rnnt) в {}", model_dir)
        }
    }

    pub fn engine_name(&self) -> &'static str {
        match self {
            Self::Ctc(_) => "CTC",
            Self::Rnnt(_) => "RNNT",
        }
    }

    pub fn recognize_file(
        &self,
        path: &str,
        cancel: &Arc<AtomicBool>,
    ) -> Result<String> {
        match self {
            Self::Ctc(model) => model.recognize_file(path, cancel),
            Self::Rnnt(model) => model.recognize_file(path, cancel),
        }
    }
}

// -----------------------------------------------------------------------------
// RNN-T (GigaAM v3 E2E RNNT) — по эталону onnx-asr asr.py / models/gigaam.py
// -----------------------------------------------------------------------------
pub struct GigaamRnnt {
    encoder: Mutex<Session>,
    decoder: Mutex<Session>,
    joiner: Mutex<Session>,
    vocab: HashMap<u32, String>,
    blank_idx: u32,
    preprocessor: Preprocessor,
}

impl GigaamRnnt {
    pub fn load(model_dir: &str) -> Result<Self> {
        let enc_path = find_file(model_dir, "rnnt_encoder")?;
        let dec_path = find_file(model_dir, "rnnt_decoder")?;
        let joint_path = find_file(model_dir, "rnnt_joint")?;
        let vocab_path = format!("{model_dir}\\v3_e2e_rnnt_vocab.txt");

        let encoder = Session::builder()?.commit_from_file(&enc_path)?;
        let decoder = Session::builder()?.commit_from_file(&dec_path)?;
        let joiner = Session::builder()?.commit_from_file(&joint_path)?;

        let (vocab, blank_idx) = load_vocab_auto(&vocab_path)?;

        Ok(Self {
            encoder: Mutex::new(encoder),
            decoder: Mutex::new(decoder),
            joiner: Mutex::new(joiner),
            vocab,
            blank_idx,
            preprocessor: Preprocessor::new(model_dir),
        })
    }

    pub fn recognize_file(
        &self,
        path: &str,
        cancel: &Arc<AtomicBool>,
    ) -> Result<String> {
        let (mono, rate) = decode_to_mono(path)?;
        let mono16k = if rate == 16000 { mono } else { resample_linear(&mono, rate, 16000) };
        let chunks = crate::modules::asr::vad::split_by_length(&mono16k, 16000, 25.0);
        let mut all_text = Vec::new();

        let mut encoder_guard = self.encoder.lock().unwrap();
        let mut decoder_guard = self.decoder.lock().unwrap();
        let mut joiner_guard = self.joiner.lock().unwrap();

        for chunk in chunks {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            let (features, real_frames) = self.preprocessor.compute(&chunk);
            if real_frames == 0 {
                continue;
            }

            use ndarray::Array;
            // Эталон: features имеет форму (B, 64, T)
            let features_array = Array::from_shape_vec((1, 64, real_frames), features)?;
            let lengths_array = Array::from_shape_vec((1,), vec![real_frames as i64])?;

            // 1. Encoder: audio_signal (B x 64 x T), length (B)
            let enc_outputs = encoder_guard.run(ort::inputs! {
                "audio_signal" => ort::value::TensorRef::from_array_view(&features_array)?,
                "length" => ort::value::TensorRef::from_array_view(&lengths_array)?,
            })?;

            let (enc_shape, enc_slice) = enc_outputs["encoded"].try_extract_tensor::<f32>()?;
            let enc_shape_t = (
                enc_shape[0] as usize,
                enc_shape[1] as usize,
                enc_shape[2] as usize,
            );
            // onnx_asr: encoder_out.transpose(0, 2, 1) -> (B, T, Dim)
            let encoded = ndarray::ArrayView3::from_shape(enc_shape_t, enc_slice)?
                .permuted_axes([0, 2, 1])
                .to_owned();

            // Длина выхода encoder (subsampling=4): (real_frames - 1) // 4 + 1
            let t_len = ((real_frames as i64 - 1) / 4 + 1) as usize;
            let t_len = t_len.min(enc_shape_t.2);

            // 2. Decoder + Joint (transducer greedy decode, эталон asr.py)
            let mut h = ndarray::Array3::<f32>::zeros((1, 1, 320));
            let mut c = ndarray::Array3::<f32>::zeros((1, 1, 320));
            let mut next_h = h.clone();
            let mut next_c = c.clone();

            let mut dec_out = ndarray::Array3::<f32>::zeros((1, 1, 320));
            let mut dec_needs_run = true;

            let mut tokens: Vec<i64> = Vec::new();
            let mut t = 0usize;
            let mut emitted_tokens = 0usize;
            let max_tokens_per_step = 3usize;

            while t < t_len {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                if dec_needs_run {
                    let x_val = tokens.last().copied().unwrap_or(self.blank_idx as i64);
                    let x = ndarray::Array2::from_elem((1, 1), x_val);

                    let dec_res = decoder_guard.run(ort::inputs! {
                        "x" => ort::value::TensorRef::from_array_view(&x)?,
                        "h.1" => ort::value::TensorRef::from_array_view(&h)?,
                        "c.1" => ort::value::TensorRef::from_array_view(&c)?,
                    })?;

                    let (d_shape, d_slice) = dec_res["dec"].try_extract_tensor::<f32>()?;
                    let d_shape_t = (d_shape[0] as usize, d_shape[1] as usize, d_shape[2] as usize);
                    dec_out = ndarray::ArrayView3::from_shape(d_shape_t, d_slice)?.to_owned();

                    let (h_shape, h_slice) = dec_res["h"].try_extract_tensor::<f32>()?;
                    let h_shape_t = (h_shape[0] as usize, h_shape[1] as usize, h_shape[2] as usize);
                    next_h = ndarray::ArrayView3::from_shape(h_shape_t, h_slice)?.to_owned();

                    let (c_shape, c_slice) = dec_res["c"].try_extract_tensor::<f32>()?;
                    let c_shape_t = (c_shape[0] as usize, c_shape[1] as usize, c_shape[2] as usize);
                    next_c = ndarray::ArrayView3::from_shape(c_shape_t, c_slice)?.to_owned();

                    dec_needs_run = false;
                }

                // enc_t: encoder_out[:, t, :] -> транспонируем в (1, Dim, 1),
                // т.к. joiner ждёт enc в форме (B, Dim, T).
                let enc_t = encoded
                    .slice(ndarray::s![.., t..t + 1, ..])
                    .permuted_axes([0, 2, 1])
                    .to_owned();
                // dec должен быть (1, Dim, 1) — joiner ждёт dec в (B, Dim, T).
                let dec_t = dec_out.clone().permuted_axes([0, 2, 1]);
                let joint_res = joiner_guard.run(ort::inputs! {
                    "enc" => ort::value::TensorRef::from_array_view(&enc_t)?,
                    "dec" => ort::value::TensorRef::from_array_view(&dec_t)?,
                })?;

                let (_, joint_slice) = joint_res["joint"].try_extract_tensor::<f32>()?;

                let mut max_val = f32::NEG_INFINITY;
                let mut best_idx = 0usize;
                for (i, &val) in joint_slice.iter().enumerate() {
                    if val > max_val {
                        max_val = val;
                        best_idx = i;
                    }
                }

                if best_idx as u32 != self.blank_idx {
                    h = next_h.clone();
                    c = next_c.clone();
                    tokens.push(best_idx as i64);
                    emitted_tokens += 1;
                    dec_needs_run = true;
                }

                if best_idx as u32 == self.blank_idx || emitted_tokens == max_tokens_per_step {
                    t += 1;
                    emitted_tokens = 0;
                }
            }

            let ids: Vec<u32> = tokens.iter().map(|&i| i as u32).collect();
            let text = detokenize(&ids, &self.vocab);
            if !text.is_empty() {
                all_text.push(text);
            }
        }

        Ok(all_text.join(" "))
    }
}

// -----------------------------------------------------------------------------
// CTC (GigaAM v3 E2E CTC) — по эталону onnx-asr asr.py (_AsrWithCtcDecoding)
// -----------------------------------------------------------------------------
pub struct GigaamCtc {
    session: Mutex<Session>,
    vocab: HashMap<u32, String>,
    blank_idx: u32,
    preprocessor: Preprocessor,
}

impl GigaamCtc {
    pub fn load(model_dir: &str) -> Result<Self> {
        let model_path = find_file(model_dir, "ctc")?;
        let vocab_path = format!("{model_dir}\\v3_e2e_ctc_vocab.txt");

        let session = Session::builder()?.commit_from_file(&model_path)?;
        let (vocab, blank_idx) = load_vocab_auto(&vocab_path)?;

        Ok(Self {
            session: Mutex::new(session),
            vocab,
            blank_idx,
            preprocessor: Preprocessor::new(model_dir),
        })
    }

    pub fn recognize_file(
        &self,
        path: &str,
        cancel: &Arc<AtomicBool>,
    ) -> Result<String> {
        let (mono, rate) = decode_to_mono(path)?;
        let mono16k = if rate == 16000 { mono } else { resample_linear(&mono, rate, 16000) };
        let chunks = crate::modules::asr::vad::split_by_length(&mono16k, 16000, 25.0);
        let mut all_text = Vec::new();

        let mut session_guard = self.session.lock().unwrap();

        for chunk in chunks {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            let (features, real_frames) = self.preprocessor.compute(&chunk);
            if real_frames == 0 {
                continue;
            }

            use ndarray::Array;
            // Эталон: features (B x 64 x T), feature_lengths (B)
            let features_array = Array::from_shape_vec((1, 64, real_frames), features)?;
            let lengths_array = Array::from_shape_vec((1,), vec![real_frames as i64])?;

            let outputs = session_guard.run(ort::inputs! {
                "features" => ort::value::TensorRef::from_array_view(&features_array)?,
                "feature_lengths" => ort::value::TensorRef::from_array_view(&lengths_array)?,
            })?;

            // Выход: log_probs (B, T, vocab)
            let (lp_shape, lp_slice) = outputs["log_probs"].try_extract_tensor::<f32>()?;
            let t = lp_shape[1] as usize;
            let v = lp_shape[2] as usize;

            // CTC-decode (эталон asr.py _AsrWithCtcDecoding):
            // argmax по vocab, маска != blank, схлопывание соседних повторов.
            let mut decoded = Vec::new();
            let mut prev = self.blank_idx;
            for tt in 0..t {
                let mut best = 0u32;
                let mut best_val = f32::NEG_INFINITY;
                for k in 0..v {
                    let val = lp_slice[tt * v + k];
                    if val > best_val {
                        best_val = val;
                        best = k as u32;
                    }
                }
                if best != self.blank_idx && best != prev {
                    decoded.push(best);
                }
                prev = best;
            }

            let text = detokenize(&decoded, &self.vocab);
            if !text.is_empty() {
                all_text.push(text);
            }
        }
        Ok(all_text.join(" "))
    }
}

// -----------------------------------------------------------------------------
// Утилиты
// -----------------------------------------------------------------------------
fn detokenize(ids: &[u32], vocab: &HashMap<u32, String>) -> String {
    let tokens: Vec<String> = ids.iter().filter_map(|&i| vocab.get(&i).cloned()).collect();
    tokens.concat().replace('\u{2581}', " ").trim().to_string()
}

/// Загружает vocab и определяет blank_idx автоматически (поддержка форматов
/// "token id" и "id token").
fn load_vocab_auto(path: &str) -> Result<(HashMap<u32, String>, u32)> {
    let content = std::fs::read_to_string(path).context(format!("не найден vocab: {path}"))?;
    let mut map = HashMap::new();
    for line in content.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            continue;
        }
        // Формат onnx-asr: "token id" (слово + число). Поддержим оба порядка.
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() == 2 {
            if let Ok(idx) = parts[1].parse::<u32>() {
                map.insert(idx, parts[0].to_string());
            } else if let Ok(idx) = parts[0].parse::<u32>() {
                map.insert(idx, parts[1].to_string());
            }
        }
    }
    if map.is_empty() {
        anyhow::bail!("vocab пуст или не распознан: {path}");
    }
    let blank_idx = map
        .iter()
        .find(|(_, t)| t.as_str() == "<blk>" || t.as_str() == "<blank>")
        .map(|(i, _)| *i)
        .unwrap_or(0);
    Ok((map, blank_idx))
}

fn find_file(dir: &str, pattern: &str) -> Result<String> {
    find_file_opt(dir, pattern)
        .context(format!("Не найден файл с паттерном {pattern} в {dir}"))
}

fn find_file_opt(dir: &str, pattern: &str) -> Option<String> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .find(|e| e.file_name().to_string_lossy().contains(pattern))
        .map(|e| e.path().to_string_lossy().to_string())
}

fn resample_linear(samples: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to {
        return samples.to_vec();
    }
    let ratio = to as f64 / from as f64;
    let new_len = (samples.len() as f64 * ratio).round() as usize;
    let mut out = vec![0.0f32; new_len];
    for i in 0..new_len {
        let src = i as f64 / ratio;
        let i0 = src.floor() as usize;
        let i1 = (i0 + 1).min(samples.len() - 1);
        let frac = src - i0 as f64;
        out[i] = samples[i0] * (1.0 - frac) as f32 + samples[i1] * frac as f32;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model_dir() -> String {
        // Дефолтная папка модели из GUI
        "D:\\nn\\models\\stt\\gigaam-v3".to_string()
    }

    #[test]
    fn preprocessor_v3_not_empty() {
        let pre = Preprocessor::new(&model_dir());
        // Синтетический сигнал 1 сек @16k (не тишина)
        let sr = 16000usize;
        let mut wav = vec![0.0f32; sr];
        for (i, s) in wav.iter_mut().enumerate() {
            *s = (2.0 * std::f32::consts::PI * 220.0 * i as f32 / sr as f32).sin() * 0.3;
        }
        let (features, frames) = pre.compute(&wav);
        assert!(frames > 0, "число фреймов должно быть > 0");
        assert_eq!(features.len(), 64 * frames, "features должен быть формы 64 x frames");
        // Никаких NaN/inf
        assert!(
            features.iter().all(|&x| x.is_finite()),
            "признаки не должны содержать NaN/inf"
        );
        // Лог-мел должен давать отрицательные или около нуля значения, не +inf
        let max_v = features.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        assert!(max_v.is_finite(), "максимум признаков должен быть конечным");
        println!("preprocessor_v3: frames={frames}, max_feature={max_v:.3}");
    }

    #[test]
    fn recognize_real_ogg_not_empty() {
        let dir = model_dir();
        if !Path::new(&dir).exists() {
            eprintln!("⚠️ папка модели не найдена: {dir} — пропускаем");
            return;
        }
        let test_file = "E:\\Downloads\\audio_2026-07-18_23-59-01.ogg";
        if !Path::new(test_file).exists() {
            eprintln!("⚠️ тестовый файл не найден: {test_file} — пропускаем");
            return;
        }

        let runner = ModelRunner::load(&dir).expect("модель должна загрузиться");
        println!("движок: {}", runner.engine_name());

        let cancel = Arc::new(AtomicBool::new(false));
        let text = runner
            .recognize_file(test_file, &cancel)
            .expect("распознавание не должно падать");

        println!("распознанный текст: '{text}'");
        assert!(!text.is_empty(), "текст НЕ должен быть пустым для реального аудио");
    }
}


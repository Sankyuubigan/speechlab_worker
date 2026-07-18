use std::collections::HashMap;
use std::path::Path;
use anyhow::{Context, Result};
use ort::session::Session;
use std::sync::Mutex;

use crate::modules::audio::decode_to_mono;
use crate::modules::asr::logmel::Preprocessor;

/// Обертка, позволяющая загрузить любую из двух моделей (CTC или RNNT)
pub enum ModelRunner {
    Ctc(GigaamCtc),
    Rnnt(GigaamRnnt),
}

impl ModelRunner {
    pub fn load(model_dir: &str) -> Result<Self> {
        let rnnt_enc = format!("{model_dir}\\v3_e2e_rnnt_encoder.int8.onnx");
        let ctc_mod = format!("{model_dir}\\v3_e2e_ctc.int8.onnx");
        
        if Path::new(&rnnt_enc).exists() {
            Ok(Self::Rnnt(GigaamRnnt::load(model_dir)?))
        } else if Path::new(&ctc_mod).exists() {
            Ok(Self::Ctc(GigaamCtc::load(model_dir)?))
        } else {
            // fallbacks without int8 suffix
            let rnnt_enc_fp = format!("{model_dir}\\v3_e2e_rnnt_encoder.onnx");
            if Path::new(&rnnt_enc_fp).exists() {
                Ok(Self::Rnnt(GigaamRnnt::load(model_dir)?))
            } else {
                anyhow::bail!("Не найдены подходящие файлы onnx в {}", model_dir);
            }
        }
    }

    pub fn recognize_file(&self, path: &str) -> Result<String> {
        match self {
            Self::Ctc(model) => model.recognize_file(path),
            Self::Rnnt(model) => model.recognize_file(path),
        }
    }
}

// -----------------------------------------------------------------------------
// RNN-T (Реализация для gigaam-v3-e2e-rnnt)
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

        let vocab = load_vocab(&vocab_path)?;
        let blank_idx = vocab.iter()
            .find(|(_, t)| t.as_str() == "<blk>" || t.as_str() == "<blank>")
            .map(|(i, _)| *i).unwrap_or(0);

        Ok(Self { 
            encoder: Mutex::new(encoder), 
            decoder: Mutex::new(decoder), 
            joiner: Mutex::new(joiner), 
            vocab, 
            blank_idx, 
            preprocessor: Preprocessor::new(model_dir) 
        })
    }

    pub fn recognize_file(&self, path: &str) -> Result<String> {
        let (mono, rate) = decode_to_mono(path)?;
        let mono16k = if rate == 16000 { mono } else { resample_linear(&mono, rate, 16000) };
        
        let chunks = crate::modules::asr::vad::split_by_length(&mono16k, 16000, 25.0);
        let mut all_text = Vec::new();

        let mut encoder_guard = self.encoder.lock().unwrap();
        let mut decoder_guard = self.decoder.lock().unwrap();
        let mut joiner_guard = self.joiner.lock().unwrap();

        for chunk in chunks {
            let (features, real_frames) = self.preprocessor.compute(&chunk);
            if real_frames == 0 { continue; }

            use ndarray::Array;
            // Убрал цикл с перемешиванием, передаем features напрямую
            let features_array = Array::from_shape_vec((1, 64, real_frames), features)?;
            let lengths_array = Array::from_shape_vec((1,), vec![real_frames as i64])?;

            // 1. Encoder 
            let enc_outputs = encoder_guard.run(ort::inputs! {
                "audio_signal" => ort::value::TensorRef::from_array_view(&features_array)?,
                "length" => ort::value::TensorRef::from_array_view(&lengths_array)?,
            })?;
            
            let (enc_shape, enc_slice) = enc_outputs["encoded"].try_extract_tensor::<f32>()?;
            let enc_shape_t = (enc_shape[0] as usize, enc_shape[1] as usize, enc_shape[2] as usize);
            let encoded = ndarray::ArrayView3::from_shape(enc_shape_t, enc_slice)?;
            
            // В зависимости от формата [1, Dim, Time] или [1, Time, Dim]
            let t_len = enc_shape_t.2;

            // 2. Decoder + Joint (Transducer loop)
            let mut h = ndarray::Array3::<f32>::zeros((1, 1, 320));
            let mut c = ndarray::Array3::<f32>::zeros((1, 1, 320));
            let mut next_h = h.clone();
            let mut next_c = c.clone();
            
            let mut dec_out = ndarray::Array3::<f32>::zeros((1, 1, 320));
            let mut dec_needs_run = true;

            let mut tokens = Vec::new();
            let mut t = 0;
            let mut emitted_tokens = 0;
            let max_tokens_per_step = 3;

            while t < t_len {
                if dec_needs_run {
                    let x_val = tokens.last().copied().unwrap_or(self.blank_idx) as i64;
                    let x = ndarray::Array2::from_elem((1, 1), x_val);
                    
                    let dec_res = decoder_guard.run(ort::inputs!{
                        "x" => ort::value::TensorRef::from_array_view(&x)?, 
                        "h.1" => ort::value::TensorRef::from_array_view(&h)?, 
                        "c.1" => ort::value::TensorRef::from_array_view(&c)?
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

                let enc_t = encoded.slice(ndarray::s![.., .., t..t+1]).to_owned(); 
                
                let d_shape = dec_out.shape();
                let dec_transposed = if d_shape[1] == 1 && d_shape[2] != 1 {
                    dec_out.clone().permuted_axes([0, 2, 1])
                } else {
                    dec_out.clone()
                };

                let joint_res = joiner_guard.run(ort::inputs! {
                    "enc" => ort::value::TensorRef::from_array_view(&enc_t)?,
                    "dec" => ort::value::TensorRef::from_array_view(&dec_transposed)?,
                })?;
                
                let (_, joint_slice) = joint_res["joint"].try_extract_tensor::<f32>()?;

                let mut max_val = f32::NEG_INFINITY;
                let mut best_idx = 0;
                for (i, &val) in joint_slice.iter().enumerate() {
                    if val > max_val {
                        max_val = val;
                        best_idx = i as u32;
                    }
                }

                if best_idx != self.blank_idx {
                    h = next_h.clone();
                    c = next_c.clone();
                    tokens.push(best_idx);
                    emitted_tokens += 1;
                    dec_needs_run = true;
                }

                if best_idx == self.blank_idx || emitted_tokens == max_tokens_per_step {
                    t += 1;
                    emitted_tokens = 0;
                }
            }
            
            let text = detokenize(&tokens, &self.vocab);
            if !text.is_empty() {
                all_text.push(text);
            }
        }

        Ok(all_text.join(" "))
    }
}

// -----------------------------------------------------------------------------
// CTC (Оригинальная fallback-версия для gigaam-v3-e2e-ctc)
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
        let vocab = load_vocab(&vocab_path)?;
        let blank_idx = vocab.iter()
            .find(|(_, t)| t.as_str() == "<blk>" || t.as_str() == "<blank>")
            .map(|(i, _)| *i).unwrap_or(0);

        Ok(Self { 
            session: Mutex::new(session), 
            vocab, 
            blank_idx, 
            preprocessor: Preprocessor::new(model_dir) 
        })
    }

    pub fn recognize_file(&self, path: &str) -> Result<String> {
        let (mono, rate) = decode_to_mono(path)?;
        let mono16k = if rate == 16000 { mono } else { resample_linear(&mono, rate, 16000) };
        let chunks = crate::modules::asr::vad::split_by_length(&mono16k, 16000, 25.0);
        let mut all_text = Vec::new();

        let mut session_guard = self.session.lock().unwrap();

        for chunk in chunks {
            let (features, real_frames) = self.preprocessor.compute(&chunk);
            if real_frames == 0 { continue; }

            use ndarray::Array;
            // Убрал цикл с перемешиванием, передаем features напрямую
            let features_array = Array::from_shape_vec((1, 64, real_frames), features)?;
            let lengths_array = Array::from_shape_vec((1,), vec![real_frames as i64])?;

            let outputs = session_guard.run(ort::inputs! {
                "features" => ort::value::TensorRef::from_array_view(&features_array)?,
                "feature_lengths" => ort::value::TensorRef::from_array_view(&lengths_array)?,
            })?;

            let (lp_shape, lp_slice) = outputs["log_probs"].try_extract_tensor::<f32>()?;
            
            // Безопасное определение Time и Vocab размерностей
            let (t, v) = if lp_shape.len() == 3 {
                if lp_shape[0] == 1 {
                    (lp_shape[1] as usize, lp_shape[2] as usize)
                } else {
                    (lp_shape[0] as usize, lp_shape[2] as usize)
                }
            } else {
                (lp_shape[0] as usize, lp_shape[1] as usize)
            };

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

fn load_vocab(path: &str) -> Result<HashMap<u32, String>> {
    let content = std::fs::read_to_string(path).context(format!("не найден vocab: {path}"))?;
    let mut map = HashMap::new();
    for line in content.lines() {
        if let Some((token, id)) = line.trim_end().rsplit_once(' ') {
            if let Ok(idx) = id.trim().parse::<u32>() {
                map.insert(idx, token.to_string());
            }
        }
    }
    Ok(map)
}

fn find_file(dir: &str, pattern: &str) -> Result<String> {
    std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .find(|e| e.file_name().to_string_lossy().contains(pattern))
        .map(|e| e.path().to_string_lossy().to_string())
        .context(format!("Не найден файл с паттерном {} в {}", pattern, dir))
}

fn resample_linear(samples: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to { return samples.to_vec(); }
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